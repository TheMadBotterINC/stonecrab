//! The stale tracker and the acquisition loop (slice S56, ADR-0023, finding G09).
//!
//! Every test runs on the testkit's `FakeClock`: `block_on_with` moves the clock to the next
//! sleeper whenever the loop waits, so nothing sleeps on real time. The simulator is the
//! sanctioned `Driver` double (coding standards 13.5); the Modbus cases run the TCP driver
//! over the fake transport, with the wire bytes written out by hand from the MBAP layout.
#![cfg(all(
    feature = "sim",
    feature = "modbus",
    feature = "sensors",
    feature = "testkit"
))]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::{Duration, Instant, SystemTime};

    use stonecrab::modbus::{ConnectionDefaults, ModbusConnection, ModbusTcpDriver};
    use stonecrab::sensors::{Acquisition, Measurement, Quality, SensorProfile, StaleTracker};
    use stonecrab::sim::{FailureStep, SimDevice, SimDriver, SimTag};
    use stonecrab::testkit::{
        FakeClock, FakeTransportFactory, Script, Step, block_on_with, poll_once,
    };
    use stonecrab::{
        Clock, Connection, ConnectionConfig, DriverRegistry, Endpoint, ErrorKind, ItemError,
        PlcError, PlcValue, ProtocolStatus, ReadItem, ReadRequest, ReadResponse, ResourceLimits,
        Result, TagId,
    };

    const SIM_HEAD: &str = "id = \"synthetic_acquisition\"\nschema_version = 1\n\
        validation = \"synthetic\"\n[connection]\nscheme = \"sim\"\n\
        [provenance]\nsource = \"Synthetic test profile\"\n";

    const LEVEL: &str = "tank_level";
    const PUMP: &str = "pump_running";

    fn ms(count: u64) -> Duration {
        Duration::from_millis(count)
    }

    fn secs(count: u64) -> Duration {
        Duration::from_secs(count)
    }

    const NANO: Duration = Duration::from_nanos(1);

    /// A sim profile of a level and a pump state, each with extra measurement keys.
    fn sim_profile(level_keys: &str, pump_keys: &str) -> SensorProfile {
        SensorProfile::from_toml(&format!(
            "{SIM_HEAD}\
             [[measurements]]\nname = \"{LEVEL}\"\naddress = \"STATE/level:u16\"\n\
             data_type = \"UINT\"\nunit = \"mm\"\n{level_keys}\n\
             [[measurements]]\nname = \"{PUMP}\"\naddress = \"STATE/pump:bool\"\n\
             data_type = \"BOOL\"\nunit = \"1\"\n{pump_keys}\n"
        ))
        .expect("the test profile loads")
    }

    /// A simulator device `bench` behind a fake clock, with read counters on both tags.
    struct Rig {
        clock: Arc<FakeClock>,
        driver: Arc<SimDriver>,
        connection: Box<dyn Connection>,
        start: Instant,
        level_reads: Arc<AtomicU32>,
        pump_reads: Arc<AtomicU32>,
    }

    impl Rig {
        fn new(latency: Duration) -> Self {
            let device = SimDevice::new().with_latency(latency);
            let (level, level_reads) = SimTag::new(PlcValue::U16(840)).with_read_counter();
            let (pump, pump_reads) = SimTag::new(PlcValue::Bool(true)).with_read_counter();
            device.set("level", level).expect("level");
            device.set("pump", pump).expect("pump");
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
            let connection =
                block_on_with(&clock, registry.connect("sim://bench")).expect("connects");
            let start = clock.now();
            Self {
                clock,
                driver,
                connection,
                start,
                level_reads,
                pump_reads,
            }
        }

        fn acquisition<'a>(
            &'a self,
            profile: &'a SensorProfile,
            interval: Duration,
        ) -> Acquisition<'a> {
            Acquisition::new(
                profile,
                &*self.driver,
                &*self.connection,
                &*self.clock,
                interval,
            )
            .expect("the acquisition starts")
        }

        fn recv(&self, acquisition: &mut Acquisition<'_>) -> Result<Option<Measurement>> {
            block_on_with(&self.clock, acquisition.recv())
        }

        /// The next event, which must be there.
        fn next(&self, acquisition: &mut Acquisition<'_>) -> Measurement {
            self.recv(acquisition)
                .expect("no read failed")
                .expect("the acquisition goes on")
        }

        fn elapsed(&self) -> Duration {
            self.clock.now().duration_since(self.start)
        }

        fn push_failure(&self, step: FailureStep) {
            self.driver
                .device("bench")
                .expect("the device")
                .push_failure(step)
                .expect("the step is queued");
        }

        fn reads(&self) -> (u32, u32) {
            (
                self.level_reads.load(Ordering::Relaxed),
                self.pump_reads.load(Ordering::Relaxed),
            )
        }
    }

    fn id(name: &str) -> TagId {
        TagId::new(name).expect("tag id")
    }

    fn name_quality(measurement: &Measurement) -> (&str, Quality) {
        (measurement.name.as_str(), measurement.quality)
    }

    // ---- The tracker on its own: hand-built responses, no connection. ----

    /// The tracker profile: a level with a range, raw retention and a 5 s threshold, and a
    /// pump state with no threshold.
    fn tracker_profile() -> SensorProfile {
        sim_profile(
            "range = { min = 0.0, max = 500.0 }\nkeep_raw = true\nstale_after_ms = 5000",
            "",
        )
    }

    /// The measurements of one hand-built response of `items`, sampled at `at`.
    fn measured(profile: &SensorProfile, items: Vec<ReadItem>, at: Instant) -> Vec<Measurement> {
        let tags = profile.tags(&SimDriver::new()).expect("tags");
        let request = tags
            .iter()
            .filter(|tag| items.iter().any(|item| item.tag() == tag.id()))
            .fold(
                ReadRequest::builder(ResourceLimits::builder().build().expect("limits")),
                |request, tag| request.tag(tag.clone()),
            )
            .build()
            .expect("request");
        let response = items
            .into_iter()
            .fold(ReadResponse::builder(&request, at, at), |response, item| {
                response.item(item)
            })
            .build()
            .expect("response");
        let wall = SystemTime::UNIX_EPOCH
            .checked_add(secs(7))
            .expect("a wall time");
        profile.measurements(&response, wall)
    }

    fn level_at(value: u16, at: Instant) -> ReadItem {
        let source = SystemTime::UNIX_EPOCH
            .checked_add(secs(3))
            .expect("a source time");
        ReadItem::new(id(LEVEL), Ok(PlcValue::U16(value)), at)
            .with_source_time(source)
            .with_status(ProtocolStatus::new("sim", 0))
    }

    fn level_failed_at(at: Instant) -> ReadItem {
        let error = ItemError::new(PlcError::new(ErrorKind::Timeout, "no reply"));
        ReadItem::new(id(LEVEL), Err(error), at)
    }

    fn record_all(tracker: &mut StaleTracker, measurements: Vec<Measurement>) {
        for measurement in measurements {
            tracker.record(measurement);
        }
    }

    // covers: SPEC mandatory-failure-cases / "stale last-known values"
    // covers: SENSORS-STALE / the threshold is exclusive: fresh at stale_after, stale after it
    #[test]
    fn a_retained_value_is_fresh_at_exactly_stale_after_and_stale_one_nanosecond_later() {
        let profile = tracker_profile();
        let mut tracker = StaleTracker::new(&profile);
        let read_at = Instant::now();
        assert_eq!(tracker.next_expiry(), None, "nothing retained");
        record_all(
            &mut tracker,
            measured(&profile, vec![level_at(840, read_at)], read_at),
        );
        let threshold = read_at + secs(5);
        assert_eq!(
            tracker.next_expiry(),
            Some(threshold + NANO),
            "the first stale instant"
        );
        assert!(tracker.expire(read_at).is_empty(), "fresh when read");
        assert!(
            tracker.expire(threshold).is_empty(),
            "an age equal to stale_after is still fresh"
        );
        let stale = tracker.expire(threshold + NANO);
        assert_eq!(stale.len(), 1, "one transition: {stale:?}");
        assert_eq!(
            stale.first().map(name_quality),
            Some((LEVEL, Quality::Stale))
        );
        assert!(
            tracker.expire(threshold + secs(60)).is_empty(),
            "a transition is reported once"
        );
        assert_eq!(
            tracker.next_expiry(),
            None,
            "a stale value cannot go stale again"
        );
    }

    // covers: SENSORS-STALE / a stale value keeps everything it was read with
    #[test]
    fn a_stale_value_keeps_its_value_times_status_and_earlier_quality() {
        let profile = tracker_profile();
        let mut tracker = StaleTracker::new(&profile);
        let read_at = Instant::now();
        // 840 mm is outside the 0..=500 range, so it was read `Uncertain`.
        let fresh = measured(&profile, vec![level_at(840, read_at)], read_at);
        let first = fresh.first().cloned().expect("a measurement");
        assert_eq!(first.quality, Quality::Uncertain, "out of range");
        assert_eq!(
            first.quality_before_stale, None,
            "a fresh reading is not stale"
        );
        record_all(&mut tracker, fresh);

        let stale = tracker.expire(read_at + secs(6));
        let marked = stale.first().cloned().expect("the transition");
        assert_eq!(marked.quality, Quality::Stale);
        assert_eq!(marked.quality_before_stale, Some(Quality::Uncertain));
        let mut unmarked = marked;
        unmarked.quality = first.quality;
        unmarked.quality_before_stale = None;
        assert_eq!(
            unmarked, first,
            "value, raw, unit, origin, both receipt times, source time and status are kept"
        );
        assert_eq!(first.received_mono, read_at, "never re-stamped");
    }

    // covers: SENSORS-STALE / a failed read does not erase the retained value
    #[test]
    fn a_reading_without_a_value_or_of_another_profile_erases_nothing() {
        let profile = tracker_profile();
        let mut tracker = StaleTracker::new(&profile);
        let read_at = Instant::now();
        record_all(
            &mut tracker,
            measured(&profile, vec![level_at(120, read_at)], read_at),
        );
        let later = read_at + secs(4);
        let failed = measured(&profile, vec![level_failed_at(later)], later);
        assert_eq!(
            failed
                .first()
                .map(|failed| (failed.quality, failed.value.is_none())),
            Some((Quality::Bad, true)),
            "a failed item is Bad without a value"
        );
        record_all(&mut tracker, failed);

        let other = SensorProfile::from_toml(&format!(
            "{SIM_HEAD}[[measurements]]\nname = \"other_level\"\naddress = \"STATE/other:u16\"\n\
             data_type = \"UINT\"\nunit = \"mm\"\n"
        ))
        .expect("the other profile loads");
        let foreign = ReadItem::new(id("other_level"), Ok(PlcValue::U16(1)), later);
        record_all(&mut tracker, measured(&other, vec![foreign], later));

        assert_eq!(
            tracker.next_expiry(),
            Some(read_at + secs(5) + NANO),
            "the value read first keeps its age"
        );
        let stale = tracker.expire(read_at + secs(5) + NANO);
        assert_eq!(
            stale
                .first()
                .map(|stale| (stale.value.clone(), stale.received_mono)),
            Some((Some(PlcValue::U16(120)), read_at)),
            "the retained value goes stale"
        );
    }

    // covers: SENSORS-STALE / a fresh value replaces a stale one; no threshold means never stale
    #[test]
    fn a_fresh_reading_replaces_a_stale_one_and_no_threshold_never_goes_stale() {
        let profile = tracker_profile();
        let mut tracker = StaleTracker::new(&profile);
        let read_at = Instant::now();
        let pump = ReadItem::new(id(PUMP), Ok(PlcValue::Bool(true)), read_at);
        record_all(
            &mut tracker,
            measured(&profile, vec![level_at(120, read_at), pump], read_at),
        );
        let stale = tracker.expire(read_at + secs(3600));
        assert_eq!(
            stale.iter().map(name_quality).collect::<Vec<_>>(),
            [(LEVEL, Quality::Stale)],
            "the pump has no stale_after"
        );

        let again = read_at + secs(3601);
        record_all(
            &mut tracker,
            measured(&profile, vec![level_at(121, again)], again),
        );
        assert_eq!(
            tracker.next_expiry(),
            Some(again + secs(5) + NANO),
            "fresh again"
        );
        assert!(
            tracker.expire(again + secs(5)).is_empty(),
            "fresh until its threshold"
        );
        assert!(
            format!("{tracker:?}").contains("StaleTracker"),
            "Debug names the type"
        );
    }

    // ---- The acquisition loop on the simulator. ----

    // covers: SENSORS-ACQUISITION / the first acquisition reads every measurement at once
    #[test]
    fn the_first_acquisition_reads_every_measurement_in_one_request() {
        let rig = Rig::new(ms(10));
        let profile = sim_profile("min_interval_ms = 1000", "min_interval_ms = 3000");
        let mut acquisition = rig.acquisition(&profile, ms(1000));
        assert_eq!(rig.reads(), (0, 0), "nothing is read before recv");

        let level = rig.next(&mut acquisition);
        let pump = rig.next(&mut acquisition);
        assert_eq!(
            [name_quality(&level), name_quality(&pump)],
            [(LEVEL, Quality::Good), (PUMP, Quality::Good)],
            "document order"
        );
        assert_eq!(level.value, Some(PlcValue::U16(840)));
        assert_eq!(rig.reads(), (1, 1));
        // One request: both items were sampled after one device latency, not two.
        assert_eq!(level.received_mono, rig.start + ms(10));
        assert_eq!(pump.received_mono, rig.start + ms(10));
        assert_eq!(
            level.received_at,
            rig.clock.wall_now(),
            "the clock's wall time when the read ended"
        );
        assert!(
            format!("{acquisition:?}").contains("synthetic_acquisition"),
            "Debug names the profile"
        );
    }

    // covers: SENSORS-ACQUISITION / a faster rate than min_interval is raised to it
    #[test]
    fn a_rate_faster_than_min_interval_is_raised_to_it() {
        let rig = Rig::new(Duration::ZERO);
        let profile = sim_profile("min_interval_ms = 1000", "");
        let mut acquisition = rig.acquisition(&profile, ms(250));
        let mut level_reads = Vec::new();
        while rig.elapsed() <= secs(3) {
            let measurement = rig.next(&mut acquisition);
            if measurement.name == LEVEL {
                level_reads.push(rig.elapsed());
            }
        }
        assert_eq!(
            level_reads,
            [Duration::ZERO, secs(1), secs(2), secs(3)],
            "the level is read every min_interval, not every 250 ms"
        );
        let (level, pump) = rig.reads();
        assert_eq!(level, 4, "four level reads in 3 s");
        assert_eq!(
            pump, 14,
            "the pump, which sets no min_interval, is read every 250 ms"
        );
    }

    // covers: SENSORS-ACQUISITION / measurements with different intervals, grouped when due together
    #[test]
    fn measurements_with_different_intervals_are_grouped_when_due_together() {
        let rig = Rig::new(ms(10));
        let profile = sim_profile("min_interval_ms = 1000", "min_interval_ms = 3000");
        let mut acquisition = rig.acquisition(&profile, ms(1000));
        let mut seen = Vec::new();
        while seen.len() < 7 {
            let measurement = rig.next(&mut acquisition);
            seen.push((
                measurement.received_mono - rig.start,
                measurement.name.clone(),
            ));
        }
        let at = |second: u64| secs(second) + ms(10);
        assert_eq!(
            seen,
            [
                (at(0), LEVEL.to_owned()),
                (at(0), PUMP.to_owned()),
                (at(1), LEVEL.to_owned()),
                (at(2), LEVEL.to_owned()),
                (at(3), LEVEL.to_owned()),
                (at(3), PUMP.to_owned()),
                (at(4), LEVEL.to_owned()),
            ],
            "each on its own interval, both in one read (one latency) when due together"
        );
        assert_eq!(rig.reads(), (5, 2));
    }

    // covers: SPEC mandatory-failure-cases / "stale last-known values"
    // covers: SENSORS-ACQUISITION / fresh to stale exactly past stale_after, cleared by a good read
    // covers: SENSORS-STALE / ages run on the monotonic clock; wall-clock jumps change nothing
    #[test]
    fn a_value_goes_stale_just_past_stale_after_and_a_good_read_clears_it() {
        let rig = Rig::new(Duration::ZERO);
        let profile = sim_profile("min_interval_ms = 10000\nstale_after_ms = 2500", "");
        let mut acquisition = rig.acquisition(&profile, secs(10));
        let fresh = rig.next(&mut acquisition);
        assert_eq!(name_quality(&fresh), (LEVEL, Quality::Good));
        // The wall clock jumps a day ahead: a wall-clock age would make the value stale now.
        let jumped = fresh
            .received_at
            .checked_add(secs(86_400))
            .expect("a wall time");
        rig.clock.set_wall_now(jumped);
        assert_eq!(
            name_quality(&rig.next(&mut acquisition)),
            (PUMP, Quality::Good)
        );

        let stale = rig.next(&mut acquisition);
        assert_eq!(
            rig.elapsed(),
            ms(2500) + NANO,
            "the first instant past stale_after"
        );
        assert_eq!(name_quality(&stale), (LEVEL, Quality::Stale));
        assert_eq!(stale.quality_before_stale, Some(Quality::Good));
        assert_eq!(
            (stale.value.clone(), stale.received_mono, stale.received_at),
            (fresh.value.clone(), fresh.received_mono, fresh.received_at),
            "the retained value, not a new reading, with the wall time it was read at"
        );
        assert_eq!(rig.reads(), (1, 1), "going stale reads nothing");

        let level = rig.next(&mut acquisition);
        assert_eq!(rig.elapsed(), secs(10), "the next read is due");
        assert_eq!(name_quality(&level), (LEVEL, Quality::Good));
        assert_eq!(level.quality_before_stale, None);
        assert_eq!(
            level.received_at,
            jumped.checked_add(secs(10)).expect("a wall time"),
            "a fresh reading carries the jumped wall time"
        );
        assert_eq!(
            name_quality(&rig.next(&mut acquisition)),
            (PUMP, Quality::Good)
        );
        // Now the wall clock jumps back to 1970: a wall-clock age would never go stale.
        rig.clock.set_wall_now(SystemTime::UNIX_EPOCH);
        let stale = rig.next(&mut acquisition);
        assert_eq!(
            (rig.elapsed(), name_quality(&stale)),
            (secs(10) + ms(2500) + NANO, (LEVEL, Quality::Stale)),
            "stale again only past the threshold of the new value"
        );
    }

    // covers: SPEC mandatory-failure-cases / "stale last-known values"
    // covers: SENSORS-ACQUISITION / failed reads keep the retained value aging to stale
    #[test]
    fn failed_reads_keep_the_retained_value_aging_to_stale() {
        let rig = Rig::new(Duration::ZERO);
        let profile = sim_profile("min_interval_ms = 1000\nstale_after_ms = 2500", "");
        let mut acquisition = rig.acquisition(&profile, secs(1));
        let fresh = rig.next(&mut acquisition);
        let _pump = rig.next(&mut acquisition);

        // At 1 s the whole read fails; at 2 s the level item fails.
        rig.push_failure(FailureStep::Backpressure);
        rig.push_failure(FailureStep::PartialFailure(vec![id(LEVEL)]));
        let error = rig
            .recv(&mut acquisition)
            .expect_err("the whole read failed");
        assert_eq!(
            (error.kind(), rig.elapsed()),
            (ErrorKind::Backpressure, secs(1))
        );
        let failed = rig.next(&mut acquisition);
        assert_eq!(rig.elapsed(), secs(2));
        assert_eq!(
            (name_quality(&failed), failed.value.clone()),
            ((LEVEL, Quality::Bad), None),
            "a failed item is delivered Bad, without a value"
        );
        let _pump = rig.next(&mut acquisition);

        let stale = rig.next(&mut acquisition);
        assert_eq!(rig.elapsed(), ms(2500) + NANO);
        assert_eq!(name_quality(&stale), (LEVEL, Quality::Stale));
        assert_eq!(
            (stale.value, stale.received_mono),
            (fresh.value, fresh.received_mono),
            "the value read at 0 s, aged"
        );
        assert_eq!(
            name_quality(&rig.next(&mut acquisition)),
            (LEVEL, Quality::Good),
            "the read at 3 s succeeds"
        );
        assert_eq!(rig.elapsed(), secs(3));
    }

    // covers: SENSORS-ACQUISITION / closing the connection ends the loop
    #[test]
    fn closing_the_connection_while_the_loop_waits_ends_it() {
        let rig = Rig::new(Duration::ZERO);
        let profile = sim_profile("", "");
        // An interval past the end of time: each measurement is read once and never due again.
        let mut acquisition = rig.acquisition(&profile, Duration::MAX);
        let _level = rig.next(&mut acquisition);
        let _pump = rig.next(&mut acquisition);
        {
            let mut waiting = acquisition.recv();
            assert!(
                poll_once(&mut waiting).is_pending(),
                "nothing is due and nothing can go stale"
            );
            block_on_with(&rig.clock, rig.connection.close()).expect("close");
            assert!(
                matches!(poll_once(&mut waiting), std::task::Poll::Ready(Ok(None))),
                "the close ends the wait at once"
            );
        }
        assert!(
            matches!(rig.recv(&mut acquisition), Ok(None)),
            "and every later call"
        );
        assert_eq!(rig.reads(), (1, 1));
    }

    // covers: SENSORS-ACQUISITION / a recv dropped while it waits loses nothing and keeps the schedule
    #[test]
    fn a_recv_dropped_while_it_waits_loses_nothing_and_keeps_the_schedule() {
        let rig = Rig::new(Duration::ZERO);
        let profile = sim_profile("min_interval_ms = 1000\nstale_after_ms = 500", "");
        let mut acquisition = rig.acquisition(&profile, secs(1));
        let _level = rig.next(&mut acquisition);
        let _pump = rig.next(&mut acquisition);
        let drop_a_wait = |acquisition: &mut Acquisition<'_>| {
            {
                let mut waiting = acquisition.recv();
                assert!(poll_once(&mut waiting).is_pending(), "nothing is due yet");
            }
            assert_eq!(
                rig.clock.sleeper_count(),
                0,
                "the dropped wait left no timer"
            );
        };
        drop_a_wait(&mut acquisition);
        rig.clock.advance(ms(250));
        drop_a_wait(&mut acquisition);
        rig.clock.advance(ms(250));
        let stale = rig.next(&mut acquisition);
        assert_eq!(
            (rig.elapsed(), name_quality(&stale)),
            (ms(500) + NANO, (LEVEL, Quality::Stale)),
            "the transition comes at its instant despite the dropped waits"
        );
        drop_a_wait(&mut acquisition);
        let level = rig.next(&mut acquisition);
        let pump = rig.next(&mut acquisition);
        assert_eq!(
            (rig.elapsed(), name_quality(&level), name_quality(&pump)),
            (secs(1), (LEVEL, Quality::Good), (PUMP, Quality::Good)),
            "the next read keeps its due time"
        );
        assert_eq!(rig.reads(), (2, 2), "no read was skipped or doubled");
    }

    // covers: SENSORS-ACQUISITION / a read dropped in flight is lost and next due one interval later
    #[test]
    fn a_recv_dropped_during_a_read_loses_that_read_and_keeps_its_interval() {
        let rig = Rig::new(ms(10));
        let profile = sim_profile("min_interval_ms = 1000", "");
        let mut acquisition = rig.acquisition(&profile, secs(1));
        let _level = rig.next(&mut acquisition);
        let _pump = rig.next(&mut acquisition);
        rig.clock.advance(ms(990));
        {
            let mut reading = acquisition.recv();
            assert!(
                poll_once(&mut reading).is_pending(),
                "the read at 1 s waits out the device latency"
            );
        }
        assert_eq!(rig.reads(), (1, 1), "the device served nothing");
        rig.clock.advance(ms(10));
        let level = rig.next(&mut acquisition);
        let pump = rig.next(&mut acquisition);
        assert_eq!(
            (level.received_mono - rig.start, name_quality(&level)),
            (secs(2) + ms(10), (LEVEL, Quality::Good)),
            "next read one interval after the dropped one started"
        );
        assert_eq!(name_quality(&pump), (PUMP, Quality::Good));
        assert_eq!(rig.reads(), (2, 2), "the dropped read is not made up");
    }

    // covers: SENSORS-ACQUISITION / a connection that fails ends the loop
    #[test]
    fn a_connection_that_fails_during_a_read_ends_the_loop() {
        let rig = Rig::new(Duration::ZERO);
        let profile = sim_profile("stale_after_ms = 1500", "");
        let mut acquisition = rig.acquisition(&profile, secs(1));
        let _level = rig.next(&mut acquisition);
        let _pump = rig.next(&mut acquisition);
        rig.push_failure(FailureStep::Disconnect);
        assert!(
            matches!(rig.recv(&mut acquisition), Ok(None)),
            "the failed connection ends it, without an error"
        );
        rig.clock.advance(secs(10));
        assert!(
            matches!(rig.recv(&mut acquisition), Ok(None)),
            "no stale transition after the end"
        );
    }

    // covers: SENSORS-ACQUISITION / a clock that never sleeps cannot pace the loop
    #[test]
    fn a_clock_that_never_sleeps_is_refused_when_the_loop_would_wait() {
        let device = SimDevice::new();
        device
            .set("level", SimTag::new(PlcValue::U16(1)))
            .expect("level");
        device
            .set("pump", SimTag::new(PlcValue::Bool(false)))
            .expect("pump");
        // The simulator's default clock is its RealtimeClock, whose sleeps resolve at once.
        let driver = Arc::new(
            SimDriver::new()
                .with_device("bench", device)
                .expect("device"),
        );
        let mut registry = DriverRegistry::new();
        registry
            .register(Arc::<SimDriver>::clone(&driver))
            .expect("register");
        let clock = FakeClock::default();
        let connection = block_on_with(&clock, registry.connect("sim://bench")).expect("connects");
        let realtime = stonecrab::sim::RealtimeClock::new();
        let profile = sim_profile("", "");
        let mut acquisition =
            Acquisition::new(&profile, &*driver, &*connection, &realtime, secs(60))
                .expect("starts");
        let first = block_on_with(&clock, acquisition.recv()).expect("the first read");
        assert!(first.is_some(), "the first acquisition needs no wait");
        let _pump = block_on_with(&clock, acquisition.recv()).expect("queued");
        let error = block_on_with(&clock, acquisition.recv())
            .expect_err("the loop cannot wait for a minute");
        assert_eq!(error.kind(), ErrorKind::InvalidConfig);
    }

    // covers: SENSORS-ACQUISITION / what the constructor refuses before any I/O
    #[test]
    fn a_zero_interval_an_empty_profile_and_a_foreign_profile_are_refused() {
        let rig = Rig::new(Duration::ZERO);
        let start = |profile: &SensorProfile, interval: Duration| {
            Acquisition::new(
                profile,
                &*rig.driver,
                &*rig.connection,
                &*rig.clock,
                interval,
            )
            .err()
            .map(|error| error.kind())
        };
        let profile = sim_profile("", "");
        assert_eq!(
            start(&profile, Duration::ZERO),
            Some(ErrorKind::InvalidConfig),
            "a zero interval"
        );
        let empty = SensorProfile::from_toml(SIM_HEAD).expect("an empty profile loads");
        assert_eq!(
            start(&empty, secs(1)),
            Some(ErrorKind::InvalidConfig),
            "nothing to read"
        );
        let modbus = SensorProfile::from_toml(&SIM_HEAD.replace("\"sim\"", "\"modbus-tcp\""))
            .expect("the modbus profile loads");
        assert_eq!(
            start(&modbus, secs(1)),
            Some(ErrorKind::InvalidConfig),
            "a profile of another scheme"
        );
        assert_eq!(rig.reads(), (0, 0), "no I/O");
    }

    // ---- The Modbus TCP driver over the fake transport. ----

    const MODBUS_PROFILE: &str = "id = \"synthetic_acquisition_modbus\"\nschema_version = 1\n\
        validation = \"synthetic\"\n[connection]\nscheme = \"modbus-tcp\"\n\
        address_convention = \"plc4x\"\n[provenance]\nsource = \"Synthetic test profile\"\n\
        [[measurements]]\nname = \"tank_level\"\naddress = \"holding-register:1:UINT\"\n\
        data_type = \"UINT\"\nunit = \"mm\"\nmin_interval_ms = 1000\n\
        [[measurements]]\nname = \"tank_temperature\"\naddress = \"holding-register:2:INT\"\n\
        data_type = \"INT\"\nunit = \"Cel\"\nscaling = [{ divide = 10.0 }]\n\
        min_interval_ms = 2000\n";

    /// An ADU of unit 1 as it goes on the wire.
    fn adu(txn: u16, pdu: &[u8]) -> Vec<u8> {
        let length = u16::try_from(pdu.len().saturating_add(1)).expect("a short PDU");
        let mut bytes = txn.to_be_bytes().to_vec();
        bytes.extend_from_slice(&[0, 0]);
        bytes.extend_from_slice(&length.to_be_bytes());
        bytes.push(1);
        bytes.extend_from_slice(pdu);
        bytes
    }

    fn open_modbus(
        clock: &FakeClock,
        limits: ResourceLimits,
        steps: Vec<Step>,
    ) -> ModbusConnection {
        let factory = FakeTransportFactory::new(clock.clone());
        factory.push_script(Script::strict(steps));
        let config =
            ConnectionConfig::builder("modbus-tcp", Endpoint::Custom("fake://plc".to_owned()))
                .limits(limits)
                .build()
                .expect("config");
        block_on_with(
            clock,
            ModbusConnection::open(
                config,
                Arc::new(factory),
                Arc::new(clock.clone()),
                ConnectionDefaults::default(),
            ),
        )
        .expect("open")
    }

    // covers: SENSORS-ACQUISITION / due measurements share one Modbus request on the wire
    #[test]
    fn modbus_measurements_due_together_share_one_wire_request() {
        let clock = FakeClock::default();
        let start = clock.now();
        // Registers 1 and 2 (wire 0 and 1) at 0 s and 2 s, register 1 alone at 1 s.
        let connection = open_modbus(
            &clock,
            ResourceLimits::builder().build().expect("limits"),
            vec![
                Step::Expect(adu(1, &[0x03, 0x00, 0x00, 0x00, 0x02])),
                Step::Reply(adu(1, &[0x03, 0x04, 0x03, 0x48, 0x00, 0xD7])),
                Step::Expect(adu(2, &[0x03, 0x00, 0x00, 0x00, 0x01])),
                Step::Reply(adu(2, &[0x03, 0x02, 0x03, 0x49])),
                Step::Expect(adu(3, &[0x03, 0x00, 0x00, 0x00, 0x02])),
                Step::Reply(adu(3, &[0x03, 0x04, 0x03, 0x4A, 0xFF, 0xF6])),
            ],
        );
        let profile = SensorProfile::from_toml(MODBUS_PROFILE).expect("the profile loads");
        let driver = ModbusTcpDriver::new();
        let mut acquisition = Acquisition::new(&profile, &driver, &connection, &clock, ms(500))
            .expect("the acquisition starts");
        let mut seen = Vec::new();
        while seen.len() < 5 {
            let measurement = block_on_with(&clock, acquisition.recv())
                .expect("no read failed")
                .expect("the acquisition goes on");
            seen.push((clock.now() - start, measurement.name, measurement.value));
        }
        assert_eq!(
            seen,
            [
                (secs(0), LEVEL.to_owned(), Some(PlcValue::U16(840))),
                (
                    secs(0),
                    "tank_temperature".to_owned(),
                    Some(PlcValue::F64(21.5))
                ),
                (secs(1), LEVEL.to_owned(), Some(PlcValue::U16(841))),
                (secs(2), LEVEL.to_owned(), Some(PlcValue::U16(842))),
                (
                    secs(2),
                    "tank_temperature".to_owned(),
                    Some(PlcValue::F64(-1.0))
                ),
            ]
        );
        block_on_with(&clock, connection.close()).expect("close");
        assert!(
            matches!(block_on_with(&clock, acquisition.recv()), Ok(None)),
            "the closed connection ends the loop"
        );
    }

    // covers: SENSORS-ACQUISITION / a profile that cannot be read in one request is refused
    #[test]
    fn a_profile_larger_than_one_read_is_refused_before_any_io() {
        let clock = FakeClock::default();
        let limits = ResourceLimits::builder()
            .max_items_per_request(1)
            .build()
            .expect("limits");
        let connection = open_modbus(&clock, limits, Vec::new());
        let profile = SensorProfile::from_toml(MODBUS_PROFILE).expect("the profile loads");
        let error = Acquisition::new(
            &profile,
            &ModbusTcpDriver::new(),
            &connection,
            &clock,
            secs(1),
        )
        .expect_err("two tags do not fit a one-item read");
        assert_eq!(error.kind(), ErrorKind::ResourceLimit);
    }
}
