//! The documented maps of the two test devices, replayed (slices S52d and S52e). HW1 is the
//! `AutomationDirect` STRIDE SIO-MB04THMS (Modbus TCP), HW2 the SOLO SLM1-C (Modbus RTU and
//! ASCII). Each map is a transcription of the vendor's document
//! (`fixtures/modbus/devices/*.toml`, labeled "documented map, not hardware-verified") and each
//! suite runs through the driver of the device's scheme (`ModbusTcpDriver`, or
//! `ModbusRtuDriver` for the SOLO, with RTU framing and CRC on the line its fixture gives) over
//! the fake transport and clock against the first-party responder serving that map, with the
//! vendor's limits enforced. It runs what the hardware record will run on the device: a read of
//! every documented register and coil, the request limits, the documented quality flags and
//! error codes through the device's sensor profile, and the documented write targets.
//!
//! Nothing here is evidence about a device. The refusal codes and every behavior the documents
//! leave open are the replay's assumptions, listed in `docs/hardware/`. The SOLO suite proves
//! the register map, the limits, the profile and the RTU framing of the driver at the settings
//! the manual gives; it proves nothing about the electrical line.
#![cfg(all(feature = "modbus", feature = "testkit", feature = "sensors"))]
#![forbid(unsafe_code)]

#[cfg(test)]
#[path = "support/responder.rs"]
#[expect(
    dead_code,
    reason = "the module is shared with modbus_extended.rs; these tests read and write no extended register"
)]
mod responder;

#[cfg(test)]
#[path = "support/documented_map.rs"]
mod documented_map;

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime};

    use stonecrab::modbus::{ExceptionCode, ModbusRtuDriver, ModbusTcpDriver};
    use stonecrab::sensors::{Measurement, Quality, SensorProfile, ValidationStatus};
    use stonecrab::testkit::load_provenance;
    use stonecrab::{
        Clock, Driver, ErrorKind, PlcValue, ProtocolStatus, TagId, UnknownReason, WriteOutcome,
    };

    use crate::documented_map::{DocumentedMap, Rig, rtu_frame, word_value};

    /// The label every documented map, profile and record carries.
    const LABEL: &str = "documented map, not hardware-verified";

    fn workspace() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn profile(relative: &str) -> SensorProfile {
        let path = workspace().join("profiles/documented").join(relative);
        let text = std::fs::read_to_string(&path).expect("the profile is readable");
        SensorProfile::from_toml(&text).expect("the profile loads")
    }

    /// The PDU of a request of function 15 or 16 for `quantity` items, all zero.
    fn multiple_write(function: u8, start: u16, quantity: u16) -> Vec<u8> {
        let data = if function == 16 {
            usize::from(quantity).saturating_mul(2)
        } else {
            usize::from(quantity).div_ceil(8)
        };
        let mut pdu = vec![function];
        pdu.extend_from_slice(&start.to_be_bytes());
        pdu.extend_from_slice(&quantity.to_be_bytes());
        pdu.push(u8::try_from(data).expect("a byte count"));
        pdu.resize(pdu.len().saturating_add(data), 0);
        pdu
    }

    /// The reply PDU of a device that refuses `function` with exception `code`.
    fn refusal(function: u8, code: u8) -> Vec<u8> {
        vec![function | 0x80, code]
    }

    /// The echo PDU a device sends for an accepted write of `quantity` items at `start`.
    fn write_echo(function: u8, start: u16, quantity: u16) -> Vec<u8> {
        let mut pdu = vec![function];
        pdu.extend_from_slice(&start.to_be_bytes());
        pdu.extend_from_slice(&quantity.to_be_bytes());
        pdu
    }

    /// The `(start, quantity)` runs of consecutive wire addresses, each run cut at `max`.
    fn runs(wires: &BTreeSet<u16>, max: u16) -> Vec<(u16, u16)> {
        let mut runs: Vec<(u16, u16)> = Vec::new();
        for wire in wires {
            match runs.last_mut() {
                Some((start, quantity))
                    if start.saturating_add(*quantity) == *wire && *quantity < max =>
                {
                    *quantity = quantity.saturating_add(1);
                }
                _ => runs.push((*wire, 1)),
            }
        }
        runs
    }

    fn function_of(area: &str) -> u8 {
        match area {
            "coil" => 1,
            "discrete-input" => 2,
            "holding-register" => 3,
            "input-register" => 4,
            other => panic!("{other} is not an area"),
        }
    }

    /// The map's own consistency, checked against the transcription rules of its header:
    /// the label, the pinned document, the provenance entry, the wire address that follows
    /// from every printed number, and the vocabulary of every field.
    fn check_transcription(
        map: &DocumentedMap,
        file: &str,
        register_wire: fn(&str) -> Option<u16>,
        coil_wire: fn(&str) -> Option<u16>,
    ) {
        assert_eq!(map.device.label, LABEL, "{file}: the label");
        assert!(
            !map.device.manufacturer.is_empty() && !map.device.model.is_empty(),
            "{file}: the device is named"
        );
        assert!(
            ["modbus-tcp", "modbus-rtu"].contains(&map.device.scheme.as_str()),
            "{file}: the scheme the device is reached by"
        );
        assert_eq!(
            map.device.address_reading, "zero-based",
            "{file}: the reading"
        );
        let source = &map.source;
        assert!(
            source
                .url
                .starts_with("https://cdn.automationdirect.com/static/manuals/")
                && !source.document.is_empty(),
            "{file}: the document is pinned by URL"
        );
        assert!(
            source.sha256.len() == 64
                && source
                    .sha256
                    .chars()
                    .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
            "{file}: the sha256 is 64 lowercase hex digits"
        );
        assert_eq!(source.retrieved, "2026-10-28", "{file}: the retrieval date");
        assert!(
            source.license.contains("vendor documentation"),
            "{file}: the document's license is noted"
        );
        let provenance = load_provenance(&workspace().join("fixtures/modbus/devices"))
            .expect("fixtures/modbus/devices/PROVENANCE.toml loads");
        let entry = provenance
            .file
            .iter()
            .find(|entry| entry.path == file)
            .unwrap_or_else(|| panic!("{file} has no [[file]] entry in devices/PROVENANCE.toml"));
        assert!(
            entry.origin.contains(&source.url) && entry.origin.contains(&source.sha256),
            "{file}: PROVENANCE.toml names the same URL and sha256"
        );
        assert!(
            entry.origin.contains(LABEL),
            "{file}: PROVENANCE.toml carries the label"
        );
        let accesses = ["RO", "R", "R/W"];
        let bases = ["documented", "default", "example", "placeholder"];
        let mut wires = BTreeSet::new();
        for register in &map.registers {
            let name = &register.name;
            assert_eq!(
                register_wire(&register.number),
                Some(register.wire),
                "{file}: {name}: the wire address follows from the printed number"
            );
            assert!(
                word_value(&register.data_type, 0).is_some(),
                "{file}: {name}: a documented type"
            );
            assert!(
                accesses.contains(&register.access.as_str())
                    && bases.contains(&register.basis.as_str()),
                "{file}: {name}: access and basis come from the vocabulary"
            );
            assert!(
                !name.trim().is_empty() && !register.meaning.trim().is_empty(),
                "{file}: {name}: a name and a meaning"
            );
            if let (Some(text), [word]) = (&register.value, register.words.as_slice()) {
                let value = word_value(&register.data_type, *word);
                let shown = if let Some(PlcValue::I16(number)) = value {
                    Some(number.to_string())
                } else if let Some(PlcValue::U16(number)) = value {
                    Some(number.to_string())
                } else {
                    None
                };
                assert_eq!(
                    shown.as_ref(),
                    Some(text),
                    "{file}: {name}: value matches words"
                );
            }
            for offset in 0..register.words.len() {
                let wire = register
                    .wire
                    .saturating_add(u16::try_from(offset).expect("a short run"));
                assert!(
                    wires.insert(wire),
                    "{file}: {name}: wire {wire} is listed once"
                );
            }
        }
        let mut coils = BTreeSet::new();
        for coil in &map.coils {
            let name = &coil.name;
            assert_eq!(
                coil_wire(&coil.number),
                Some(coil.wire),
                "{file}: {name}: the wire address follows from the printed number"
            );
            assert!(
                accesses.contains(&coil.access.as_str()) && bases.contains(&coil.basis.as_str()),
                "{file}: {name}: access and basis come from the vocabulary"
            );
            assert!(
                !name.trim().is_empty() && !coil.meaning.trim().is_empty(),
                "{file}: {name}: a name and a meaning"
            );
            assert!(coils.insert(coil.wire), "{file}: {name}: listed once");
        }
        let limits = &map.limits;
        assert!(
            !limits.functions.is_empty()
                && limits
                    .functions
                    .iter()
                    .all(|f| [1, 2, 3, 4, 5, 6, 15, 16].contains(f)),
            "{file}: the documented function codes"
        );
        for (limit, protocol_max) in [
            (limits.read_bits, 2_000),
            (limits.write_bits, 1_968),
            (limits.read_registers, 125),
            (limits.write_registers, 123),
        ] {
            assert!(
                limit.is_none_or(|limit| (1..=protocol_max).contains(&limit)),
                "{file}: a limit within the protocol's"
            );
        }
    }

    /// The profile of a device against its map: the label, the device named, the fixture
    /// that exists, and the addresses and tags a driver builds from it.
    fn check_profile(profile: &SensorProfile, map: &DocumentedMap, fixture: &str, tags: usize) {
        assert_eq!(
            profile.validation,
            ValidationStatus::Documented,
            "{fixture}: label"
        );
        assert!(
            profile.provenance.source.contains(LABEL)
                && profile.provenance.source.contains(&map.source.url)
                && profile.provenance.source.contains(&map.source.sha256),
            "{fixture}: the provenance names the label, the URL and the sha256"
        );
        assert_eq!(
            profile.provenance.fixtures,
            [format!("fixtures/modbus/devices/{fixture}")],
            "{fixture}: the fixture the profile is replayed against"
        );
        assert!(
            profile
                .provenance
                .fixtures
                .iter()
                .all(|path| workspace().join(path).is_file()),
            "{fixture}: the fixture exists"
        );
        assert_eq!(
            profile.device.manufacturer.as_deref(),
            Some(map.device.manufacturer.as_str()),
            "{fixture}: manufacturer"
        );
        assert_eq!(
            profile.device.model.as_deref(),
            Some(map.device.model.as_str()),
            "{fixture}: model"
        );
        assert_eq!(
            profile.connection.address_convention.as_deref(),
            Some(map.device.address_reading.as_str()),
            "{fixture}: the profile reads addresses as the map does"
        );
        assert_eq!(
            profile.validate(),
            Ok(()),
            "{fixture}: the profile validates"
        );
        assert_eq!(
            profile.connection.scheme, map.device.scheme,
            "{fixture}: the profile names the scheme the device is reached by"
        );
        let driver: Box<dyn Driver> = if map.device.scheme == "modbus-rtu" {
            Box::new(ModbusRtuDriver::new())
        } else {
            Box::new(ModbusTcpDriver::new())
        };
        assert_eq!(
            profile.validate_for(&*driver),
            Ok(()),
            "{fixture}: the driver accepts every address"
        );
        assert_eq!(
            profile.tags(&*driver).map(|built| built.len()),
            Ok(tags),
            "{fixture}: one tag per measurement and per status field"
        );
    }

    /// One read of every documented register (through each of `register_areas`) and coil
    /// (through each of `bit_areas`) with the options a user of the device would set: every
    /// value is the transcribed one, and the requests that reach the device are exactly the
    /// runs of documented addresses, cut at the documented limit, so no hole is read.
    fn replay_every_documented_item(
        map: &DocumentedMap,
        query: &str,
        register_areas: &[&str],
        bit_areas: &[&str],
    ) {
        let rig = Rig::new(map, &[], query);
        let connection = rig.connect();
        let mut tags = Vec::new();
        let mut expected = Vec::new();
        for area in register_areas {
            for (wire, data_type, value) in map.typed_words() {
                let id = format!("{area}_{wire}");
                tags.push(rig.tag(&id, &format!("{area}:{wire}:{data_type}")));
                expected.push((id, value));
            }
        }
        for area in bit_areas {
            for coil in &map.coils {
                let id = format!("{area}_{}", coil.wire);
                tags.push(rig.tag(&id, &format!("{area}:{}", coil.wire)));
                expected.push((id, PlcValue::Bool(coil.value)));
            }
        }
        let response = rig.read(&*connection, tags).expect("the read resolves");
        for (id, value) in expected {
            let item = response
                .get(&TagId::new(&id).expect("id"))
                .expect("an item");
            assert_eq!(
                item.outcome().as_ref().ok(),
                Some(&value),
                "{id}: the transcribed value"
            );
        }
        let words: BTreeSet<u16> = map.words().keys().copied().collect();
        let bits: BTreeSet<u16> = map.coils.iter().map(|coil| coil.wire).collect();
        let mut wanted = Vec::new();
        for area in register_areas {
            let max = map.limits.read_registers.unwrap_or(125);
            wanted.extend(
                runs(&words, max)
                    .into_iter()
                    .map(|(s, q)| (function_of(area), s, q)),
            );
        }
        for area in bit_areas {
            let max = map.limits.read_bits.unwrap_or(2_000);
            wanted.extend(
                runs(&bits, max)
                    .into_iter()
                    .map(|(s, q)| (function_of(area), s, q)),
            );
        }
        let mut sent = rig.read_requests();
        sent.sort_unstable();
        wanted.sort_unstable();
        assert_eq!(
            sent, wanted,
            "the requests are the documented runs and nothing between them"
        );
    }

    /// A read of `limit + 1` consecutive registers from `start`: split at the limit when the
    /// connection says so, refused whole by the device (exception 03, assumed) when it does not.
    fn assert_register_limit(map: &DocumentedMap, start: u16, limit: u16) {
        let tags = |rig: &Rig| {
            (start..=start.saturating_add(limit))
                .map(|wire| {
                    rig.tag(
                        &format!("r{wire}"),
                        &format!("holding-register:{wire}:UINT"),
                    )
                })
                .collect::<Vec<_>>()
        };
        let rig = Rig::new(map, &[], &format!("max-registers-per-request={limit}"));
        let connection = rig.connect();
        let response = rig
            .read(&*connection, tags(&rig))
            .expect("the read resolves");
        assert_eq!(response.iter_failed().count(), 0, "a split read succeeds");
        assert_eq!(
            rig.read_requests(),
            [(3, start, limit), (3, start.saturating_add(limit), 1)],
            "the limit cuts the read"
        );
        let rig = Rig::new(map, &[], "");
        let connection = rig.connect();
        let response = rig
            .read(&*connection, tags(&rig))
            .expect("the read resolves");
        assert_eq!(
            rig.read_requests(),
            [(3, start, limit.saturating_add(1))],
            "without the option the driver sends one request"
        );
        assert_eq!(
            response.iter_ok().count(),
            0,
            "and the device refuses every item of it"
        );
        for item in response.items() {
            let error = item.outcome().as_ref().expect_err("refused");
            assert_eq!(
                error.status().map(ProtocolStatus::code),
                Some(3),
                "exception 03 (assumed: the documents name no code)"
            );
        }
        assert_eq!(
            rig.device.reads(),
            0,
            "the device refused it before serving it"
        );
    }

    /// The measurement `name` of `measurements`.
    fn named<'a>(measurements: &'a [Measurement], name: &str) -> &'a Measurement {
        measurements
            .iter()
            .find(|measurement| measurement.name == name)
            .unwrap_or_else(|| panic!("a measurement {name}"))
    }

    /// Read `profile` against the replay of `map` with `overrides` replacing documented words.
    fn measure(
        profile: &SensorProfile,
        map: &DocumentedMap,
        overrides: &[(u16, u16)],
    ) -> Vec<Measurement> {
        let rig = Rig::new(map, overrides, "");
        let connection = rig.connect();
        let tags = profile.tags(&*rig.driver).expect("the profile builds tags");
        let response = rig.read(&*connection, tags).expect("the read resolves");
        profile.measurements(&response, SystemTime::UNIX_EPOCH)
    }

    /// Write each `(address, value, restore, word)` through a read-write connection, read it back
    /// from the wire and from the device (`word` is the register word the device must hold, none
    /// for a coil), then write the documented default back.
    fn assert_write_targets(
        map: &DocumentedMap,
        targets: &[(&str, PlcValue, PlcValue, Option<u16>)],
    ) {
        let rig = Rig::new(map, &[], "access=read-write");
        let connection = rig.connect();
        for (index, (address, value, restore, word)) in targets.iter().enumerate() {
            let id = format!("target_{index}");
            let wire: u16 = address
                .rsplit(':')
                .find_map(|part| part.parse().ok())
                .expect("an address ends in a number or a type");
            let logical = usize::from(wire).saturating_add(1);
            let before = rig.device.applied();
            let tag = || rig.tag(&id, address);
            let response = rig
                .write(&*connection, vec![(tag(), value.clone())])
                .expect("the write resolves");
            let outcome = response
                .get(&TagId::new(&id).expect("id"))
                .expect("a result");
            assert!(
                matches!(outcome.outcome(), WriteOutcome::Confirmed),
                "{address}: the write is confirmed"
            );
            assert_eq!(
                rig.device.applied(),
                before.saturating_add(1),
                "{address}: applied once"
            );
            match word {
                Some(word) => assert_eq!(
                    rig.device.holding(logical),
                    Some(*word),
                    "{address}: the register word"
                ),
                None => assert_eq!(
                    rig.device.coil(logical),
                    Some(*value == PlcValue::Bool(true)),
                    "{address}: the coil"
                ),
            }
            let read = rig
                .read(&*connection, vec![tag()])
                .expect("the read resolves");
            assert_eq!(
                read.items()
                    .first()
                    .and_then(|item| item.outcome().as_ref().ok()),
                Some(value),
                "{address}: the value reads back"
            );
            let response = rig
                .write(&*connection, vec![(tag(), restore.clone())])
                .expect("the restoring write resolves");
            assert_eq!(
                response.confirmed_count(),
                1,
                "{address}: the default is written back"
            );
        }
        assert_eq!(
            rig.device.applied(),
            u32::try_from(targets.len().saturating_mul(2)).expect("a short list"),
            "every write was applied once"
        );
    }

    /// The STRIDE SIO-MB04THMS, hardware record HW1.
    mod sio_mb04thms {
        use super::*;

        const FILE: &str = "sio-mb04thms.toml";
        const PROFILE: &str = "modbus-tcp/stride_sio_mb04thms.toml";
        /// The options the profile and the runbook name: the guide's 64 registers and 128 coils.
        const OPTIONS: &str = "max-registers-per-request=64&max-coils-per-request=128";

        fn map() -> DocumentedMap {
            DocumentedMap::load(FILE)
        }

        /// A register printed as 4xxxx is wire address xxxx.
        fn register_wire(number: &str) -> Option<u16> {
            number.parse::<u16>().ok()?.checked_sub(40_000)
        }

        /// A coil printed as 00161 is wire address 161 (the guide's hex column says 0x00A1).
        fn coil_wire(number: &str) -> Option<u16> {
            number.parse().ok()
        }

        // covers: MB-DEVICE-SIO-MAP / the transcription is consistent with its document
        #[test]
        fn the_map_is_a_consistent_transcription_of_the_pinned_guide() {
            let map = map();
            check_transcription(&map, FILE, register_wire, coil_wire);
            assert_eq!(map.device.unit_id, 1, "node id 40007 defaults to 1");
            assert_eq!(
                map.limits.functions,
                [1, 2, 3, 4, 5, 6, 15, 16],
                "the guide's function table"
            );
            assert_eq!(
                (map.limits.read_bits, map.limits.read_registers),
                (Some(128), Some(64)),
                "page 1: 128 coils and 64 registers a read"
            );
            assert_eq!(
                (map.limits.write_bits, map.limits.write_registers),
                (Some(64), Some(64)),
                "page 1: 64 coils and 64 registers a write"
            );
            let wires: Vec<u16> = map.words().keys().copied().collect();
            assert_eq!(
                wires,
                [
                    2, 3, 7, 11, 13, 31, 32, 33, 34, 41, 42, 43, 44, 50, 1218, 1219, 1220, 1221,
                    1241, 1242, 1243, 1244
                ],
                "the registers the guide's table lists as readable data, as wire addresses (40004 and \
                 40005, listed as Reserved, are deliberately not mapped)"
            );
            assert_eq!(map.coils.len(), 3, "coils 161 to 163");
            // The defaults the guide states, written out from the document and not from the
            // fixture (pages 3 and 4).
            let words = map.words();
            for (number, wire, default, what) in [
                ("40007", 7, 1, "node id defaults to 1"),
                (
                    "40011",
                    11,
                    0x0400,
                    "system flags: the power-up event (bit 10) is set at power-on",
                ),
                ("40013", 13, 10, "watchdog timer defaults to 10 seconds"),
                (
                    "40031",
                    31,
                    5,
                    "channel 0 input type defaults to TC K (05h)",
                ),
                (
                    "40032",
                    32,
                    5,
                    "channel 1 input type defaults to TC K (05h)",
                ),
                (
                    "40033",
                    33,
                    5,
                    "channel 2 input type defaults to TC K (05h)",
                ),
                (
                    "40034",
                    34,
                    5,
                    "channel 3 input type defaults to TC K (05h)",
                ),
                ("41218", 1218, 0, "channel 0 unit defaults to Celsius"),
                ("41219", 1219, 0, "channel 1 unit defaults to Celsius"),
                ("41220", 1220, 0, "channel 2 unit defaults to Celsius"),
                ("41221", 1221, 0, "channel 3 unit defaults to Celsius"),
            ] {
                assert_eq!(words.get(&wire).copied(), Some(default), "{number}: {what}");
            }
        }

        // covers: MB-DEVICE-SIO-MAP / every documented register and coil reads back, through
        // both function codes of each mirrored pair
        #[test]
        fn every_documented_register_and_coil_reads_back_through_both_mirrors() {
            replay_every_documented_item(
                &map(),
                OPTIONS,
                &["holding-register", "input-register"],
                &["coil", "discrete-input"],
            );
        }

        // covers: MB-DEVICE-SIO-LIMITS / 64 registers and 128 coils a read, 64 a write
        #[test]
        fn the_documented_request_limits_cut_reads_and_refuse_larger_requests() {
            let map = map();
            assert_register_limit(&map, 0, 64);
            let rig = Rig::new(&map, &[], OPTIONS);
            let connection = rig.connect();
            let coils = |rig: &Rig| {
                (33..=161)
                    .map(|wire| rig.tag(&format!("c{wire}"), &format!("coil:{wire}")))
                    .collect::<Vec<_>>()
            };
            let response = rig
                .read(&*connection, coils(&rig))
                .expect("the read resolves");
            assert_eq!(
                response.iter_failed().count(),
                0,
                "129 coils read in two requests"
            );
            assert_eq!(
                rig.read_requests(),
                [(1, 33, 128), (1, 161, 1)],
                "the limit cuts the coil read"
            );
            let rig = Rig::new(&map, &[], "");
            let connection = rig.connect();
            let response = rig
                .read(&*connection, coils(&rig))
                .expect("the read resolves");
            assert_eq!(
                response.iter_ok().count(),
                0,
                "without the option 129 coils are refused"
            );
            let mut peer = rig.peer();
            for (function, limit) in [(16, 64), (15, 64)] {
                assert_eq!(
                    peer(&multiple_write(function, 100, limit)),
                    Some(write_echo(function, 100, limit)),
                    "function {function}: {limit} items are accepted"
                );
                assert_eq!(
                    peer(&multiple_write(function, 100, limit.saturating_add(1))),
                    Some(refusal(function, 3)),
                    "function {function}: {} items are refused with exception 03 (assumed)",
                    limit.saturating_add(1)
                );
            }
        }

        // covers: MB-DEVICE-SIO-LIMITS / a function the guide does not list is refused
        #[test]
        fn an_unlisted_function_is_refused_with_exception_01() {
            let rig = Rig::new(&map(), &[], "");
            let mut peer = rig.peer();
            assert_eq!(
                peer(&[0x2B, 0x0E, 0x01, 0x00]),
                Some(refusal(0x2B, 1)),
                "function 0x2B is not in the guide's table (exception 01 is assumed)"
            );
        }

        // covers: MB-DEVICE-SIO-MAP / the profile is the map's profile
        #[test]
        fn the_profile_is_documented_and_validates_against_the_driver() {
            check_profile(&profile(PROFILE), &map(), FILE, 16);
        }

        // covers: MB-DEVICE-SIO-BREAK / a break bit makes its channel Bad and no other
        #[test]
        fn a_break_bit_makes_that_channel_bad_and_no_other() {
            let profile = profile(PROFILE);
            let map = map();
            let nominal = measure(&profile, &map, &[]);
            assert_eq!(nominal.len(), 4, "four channels");
            for (channel, (reading, raw)) in
                [(24.6, 246), (-5.2, -52), (100.0, 1000), (250.1, 2501)]
                    .into_iter()
                    .enumerate()
            {
                let measurement = named(&nominal, &format!("channel_{channel}_temperature"));
                assert_eq!(
                    measurement.quality,
                    Quality::Good,
                    "channel {channel}: no break"
                );
                assert_eq!(
                    measurement.value,
                    Some(PlcValue::F64(reading)),
                    "channel {channel}: value"
                );
                assert_eq!(
                    measurement.raw,
                    Some(PlcValue::I16(raw)),
                    "channel {channel}: raw"
                );
                assert_eq!(measurement.unit, "Cel", "channel {channel}: unit");
                assert_eq!(
                    measurement.origin,
                    ValidationStatus::Documented,
                    "channel {channel}: origin"
                );
            }
            // Break Status 40050: bit 0 channel 0, bit 2 channel 1, bit 4 channel 2, bit 6
            // channel 3, as the guide's table prints them.
            for (broken, word) in [(0, 0x0001), (1, 0x0004), (2, 0x0010), (3, 0x0040)] {
                let measurements = measure(&profile, &map, &[(50, word)]);
                for channel in 0..4 {
                    let expected = if channel == broken {
                        Quality::Bad
                    } else {
                        Quality::Good
                    };
                    let measurement =
                        named(&measurements, &format!("channel_{channel}_temperature"));
                    assert_eq!(
                        measurement.quality, expected,
                        "break word {word:#06x}: channel {channel}"
                    );
                }
            }
            let all = measure(&profile, &map, &[(50, 0x0055)]);
            assert!(
                all.iter()
                    .all(|measurement| measurement.quality == Quality::Bad),
                "all four breaks make all four channels Bad"
            );
            let unused = measure(&profile, &map, &[(50, 0x00AA)]);
            assert!(
                unused
                    .iter()
                    .all(|measurement| measurement.quality == Quality::Good),
                "the guide's unused bits (1, 3, 5, 7) break no channel"
            );
        }

        // covers: MB-DEVICE-SIO-BREAK / a unit other than Celsius is never read as Celsius
        #[test]
        fn a_degree_unit_other_than_celsius_makes_that_channel_bad() {
            let profile = profile(PROFILE);
            let map = map();
            // 41218 to 41221: 0 Celsius, 1 Fahrenheit, 2 Kelvin.
            for (wire, word, channel) in [(1218, 1, 0), (1219, 2, 1), (1221, 1, 3)] {
                let measurements = measure(&profile, &map, &[(wire, word)]);
                for other in 0..4 {
                    let expected = if other == channel {
                        Quality::Bad
                    } else {
                        Quality::Good
                    };
                    let measurement = named(&measurements, &format!("channel_{other}_temperature"));
                    assert_eq!(
                        measurement.quality, expected,
                        "units word {word} on channel {channel}: channel {other}"
                    );
                }
            }
        }

        // covers: MB-DEVICE-SIO-WRITE / the documented write targets accept a write and read back
        #[test]
        fn the_documented_write_targets_take_a_write_and_read_back() {
            // Input type (40031 to 40034), units (41218 to 41221), offsets (41241 to 41244)
            // and coils 161 to 163, each restored to the documented default after.
            assert_write_targets(
                &map(),
                &[
                    (
                        "holding-register:31:UINT",
                        PlcValue::U16(4),
                        PlcValue::U16(5),
                        Some(4),
                    ),
                    (
                        "holding-register:1218:UINT",
                        PlcValue::U16(1),
                        PlcValue::U16(0),
                        Some(1),
                    ),
                    (
                        "holding-register:1242:INT",
                        PlcValue::I16(-15),
                        PlcValue::I16(0),
                        Some(0xFFF1),
                    ),
                    (
                        "coil:161",
                        PlcValue::Bool(true),
                        PlcValue::Bool(false),
                        None,
                    ),
                    (
                        "coil:162",
                        PlcValue::Bool(true),
                        PlcValue::Bool(false),
                        None,
                    ),
                    (
                        "coil:163",
                        PlcValue::Bool(false),
                        PlcValue::Bool(true),
                        None,
                    ),
                ],
            );
        }

        // covers: MB-DEVICE-SIO-WRITE / a lost confirmation is Unknown, and the device state
        // tells what happened
        #[test]
        fn device_side_faults_end_in_typed_outcomes_and_a_lost_confirmation_is_unknown() {
            let rig = Rig::new(&map(), &[], "access=read-write");
            let connection = rig.connect();
            let status = || rig.tag("break_status", "holding-register:50:WORD");
            rig.device.arm_swallow();
            let error = rig.read(&*connection, vec![status()]).expect_err("silence");
            assert_eq!(
                error.kind(),
                ErrorKind::Timeout,
                "a silent device times out"
            );
            rig.device.arm_exception(ExceptionCode::IllegalDataAddress);
            let response = rig
                .read(&*connection, vec![status()])
                .expect("an item error");
            let refused = response
                .items()
                .first()
                .and_then(|item| item.outcome().as_ref().err());
            assert_eq!(
                refused.map(|error| (error.kind(), error.status().map(ProtocolStatus::code))),
                Some((ErrorKind::InvalidAddress, Some(2))),
                "a refusal is a typed item error carrying the exception code"
            );
            rig.device.arm_disconnect();
            let error = rig.read(&*connection, vec![status()]).expect_err("closed");
            assert_eq!(
                error.kind(),
                ErrorKind::Disconnected,
                "a closed socket is Disconnected"
            );
            let response = rig.read(&*connection, vec![status()]).expect("reconnected");
            assert_eq!(
                response
                    .items()
                    .first()
                    .and_then(|item| item.outcome().as_ref().ok()),
                Some(&PlcValue::U16(0)),
                "the next read reconnects and reads"
            );
            for (closes, reason, value) in [
                (false, UnknownReason::TimeoutAfterTransmit, true),
                (true, UnknownReason::DisconnectedAfterTransmit, false),
            ] {
                if closes {
                    rig.device.arm_apply_then_disconnect();
                } else {
                    rig.device.arm_apply_then_swallow();
                }
                let tag = rig.tag("watchdog_enable", "coil:161");
                let response = rig
                    .write(&*connection, vec![(tag, PlcValue::Bool(value))])
                    .expect("the write resolves");
                let result = response
                    .get(&TagId::new("watchdog_enable").expect("id"))
                    .expect("a result");
                assert!(
                    matches!(result.outcome(), WriteOutcome::Unknown(actual) if *actual == reason),
                    "a lost confirmation is Unknown, not rejected and not retried"
                );
                assert_eq!(
                    rig.device.coil(162),
                    Some(value),
                    "the device applied the write"
                );
            }
            assert_eq!(
                rig.device.applied(),
                2,
                "each write was applied once and never replayed"
            );
            assert_eq!(rig.device.armed(), 0, "every fault was used");
        }
    }

    /// The SOLO SLM1-C, hardware record HW2, over Modbus RTU framing on the fake transport.
    mod slm1_c {
        use super::*;

        const FILE: &str = "slm1-c.toml";
        const PROFILE: &str = "modbus-rtu/solo_slm1_c.toml";
        /// The manual's limit: eight registers a read and a write.
        const OPTIONS: &str = "max-registers-per-request=8";

        fn map() -> DocumentedMap {
            DocumentedMap::load(FILE)
        }

        /// The manual's hexadecimal address ("1000H", or "1040H~1047H": the first) is the
        /// wire address.
        fn hex_wire(number: &str) -> Option<u16> {
            let first = number.split('~').next()?;
            u16::from_str_radix(first.strip_suffix('H')?, 16).ok()
        }

        // covers: MBS-DEVICE-SLM1C-MAP / the transcription is consistent with its document
        #[test]
        fn the_map_is_a_consistent_transcription_of_the_pinned_manual() {
            let map = map();
            check_transcription(&map, FILE, hex_wire, hex_wire);
            assert_eq!(map.device.unit_id, 1, "network address 1071H defaults to 1");
            assert_eq!(
                map.limits.functions,
                [1, 2, 3, 5, 6, 16],
                "chapter 7: no 04 and no 15"
            );
            assert_eq!(
                (map.limits.read_registers, map.limits.write_registers),
                (Some(8), Some(8)),
                "FC03 and FC16 take at most eight registers"
            );
            assert_eq!(map.limits.read_bits, None, "the manual states no bit limit");
            // Every entry's first wire address, written out from the manual's tables and not
            // from the fixture (ranges count once, at their first register).
            let registers: Vec<u16> = map.registers.iter().map(|register| register.wire).collect();
            assert_eq!(
                registers,
                [
                    0x1000, 0x1001, 0x1002, 0x1003, 0x1004, 0x1005, 0x1006, 0x1007, 0x1008, 0x1009,
                    0x100A, 0x100B, 0x100C, 0x100D, 0x100E, 0x100F, 0x1010, 0x1011, 0x1012, 0x1013,
                    0x1014, 0x1015, 0x1016, 0x1019, 0x101A, 0x1020, 0x1021, 0x1023, 0x1024, 0x1025,
                    0x1026, 0x1027, 0x102A, 0x102C, 0x102E, 0x102F, 0x1030, 0x1032, 0x1033, 0x1034,
                    0x1035, 0x1036, 0x1037, 0x1038, 0x1040, 0x1050, 0x1060, 0x1068, 0x1069, 0x106A,
                    0x1071, 0x1072, 0x1073, 0x1074, 0x1075, 0x1076, 0x2000, 0x2080
                ],
                "the registers of the manual's tables, as wire addresses"
            );
            let coils: Vec<u16> = map.coils.iter().map(|coil| coil.wire).collect();
            assert_eq!(
                coils,
                [0x0811, 0x0813, 0x0814, 0x0815, 0x0816],
                "the bit registers of the manual; 0812H is not documented"
            );
            let sentinels: Vec<u16> = map.sentinels.iter().map(|sentinel| sentinel.word).collect();
            assert_eq!(
                sentinels,
                [0x8002, 0x8003, 0x8004, 0x8006, 0x8007],
                "the error codes of register 1000H; 8005H is not listed"
            );
            assert!(
                map.sentinels
                    .iter()
                    .all(|sentinel| !sentinel.meaning.is_empty()),
                "each code has its meaning"
            );
            // The appendix prints the Modbus decimal (a one-based Modicon number) beside the
            // hex of the ramp/soak registers: the decimal is 40001 plus the hex.
            let words = map.words();
            for (wire, decimal) in [
                (0x1040, 44_161),
                (0x1047, 44_168),
                (0x1050, 44_177),
                (0x1060, 44_193),
                (0x2000, 48_193),
                (0x203F, 48_256),
                (0x2080, 48_321),
                (0x20BF, 48_384),
            ] {
                assert!(words.contains_key(&wire), "{wire:#06x} is transcribed");
                assert_eq!(
                    u32::from(wire).saturating_add(40_001),
                    decimal,
                    "{wire:#06x}: the manual's hex is the wire address of its decimal"
                );
            }
            let word = |wire: u16| words.get(&wire).copied();
            assert_eq!(word(0x1004), Some(12), "input type defaults to Pt100");
            assert_eq!(word(0x1072), Some(0), "ships set to ASCII");
            assert_eq!(word(0x1074), Some(1), "ships set to 7 data bits");
            assert_eq!(word(0x1075), Some(1), "ships set to even parity");
            assert_eq!(word(0x102F), Some(0x100), "V1.00 is 0x100");
            // The line of the fixture is the one the registers give: the baud rate, the parity and
            // the stop bits as the factory defaults say; 8 data bits because RTU needs 1074H
            // written to 0 (the default 1 is 7 bits, for ASCII), and 1072H written to 1.
            let serial = map
                .device
                .serial
                .as_ref()
                .expect("the SOLO's line is documented");
            let table = |values: &[u32], wire: u16| {
                word(wire).and_then(|word| values.get(usize::from(word)).copied())
            };
            assert_eq!(
                Some(serial.baud_rate),
                table(&[2_400, 4_800, 9_600, 19_200, 38_400], 0x1073),
                "1073H: the baud rate the factory default selects"
            );
            assert_eq!(
                (serial.parity.as_str(), table(&[0, 1, 2], 0x1075)),
                ("even", Some(1)),
                "1075H: 0 none, 1 even, 2 odd; the default is even"
            );
            assert_eq!(
                (serial.stop_bits, table(&[2, 1], 0x1076)),
                (1, Some(1)),
                "1076H: 0 two stop bits, 1 one; the default is one"
            );
            assert_eq!(
                serial.data_bits, 8,
                "1074H written to 0 is 8 bits, as RTU needs"
            );
            for (wire, default, what) in [
                (0x1002, 6_000, "upper limit of the range, 0.1 degree"),
                (0x1003, 0xFF38, "lower limit of the range, -200 as 16 bit"),
                (0x1009, 476, "proportional band"),
                (0x100A, 260, "integral time"),
                (0x100B, 41, "derivative time"),
                (0x1037, 1_000, "upper limit of retransmit output"),
                (0x1068, 1, "run/stop setting is Run"),
                (0x1073, 2, "baud rate 9600"),
                (0x1076, 1, "one stop bit"),
            ] {
                assert_eq!(word(wire), Some(default), "{wire:#06x}: {what}");
            }
        }

        // covers: MBS-DEVICE-SLM1C-MAP / every documented register and coil reads back
        #[test]
        fn every_documented_register_and_coil_reads_back() {
            replay_every_documented_item(
                &map(),
                OPTIONS,
                &["holding-register"],
                &["coil", "discrete-input"],
            );
        }

        // covers: MBS-DEVICE-SLM1C-LIMITS / eight registers a read and a write
        #[test]
        fn the_documented_request_limit_cuts_reads_and_refuses_larger_requests() {
            let map = map();
            // Ramp/soak SV, 2000H to 203FH: 64 mapped registers.
            assert_register_limit(&map, 0x2000, 8);
            let rig = Rig::new(&map, &[], OPTIONS);
            let connection = rig.connect();
            let tags = (0x2000..0x2040)
                .map(|wire| rig.tag(&format!("r{wire}"), &format!("holding-register:{wire}:INT")))
                .collect();
            let response = rig.read(&*connection, tags).expect("the read resolves");
            assert_eq!(response.iter_failed().count(), 0, "64 registers read");
            assert_eq!(rig.read_requests().len(), 8, "in eight requests");
            assert!(
                rig.read_requests()
                    .iter()
                    .all(|(function, _, quantity)| *function == 3 && *quantity == 8),
                "of eight registers each"
            );
            let mut peer = rig.peer();
            assert_eq!(
                peer(&multiple_write(16, 0x2000, 8)),
                Some(write_echo(16, 0x2000, 8)),
                "function 16: eight registers are accepted"
            );
            assert_eq!(
                peer(&multiple_write(16, 0x2000, 9)),
                Some(refusal(16, 3)),
                "function 16: nine registers are refused with exception 03 (assumed)"
            );
        }

        // covers: MBS-DEVICE-SLM1C-LIMITS / functions 04 and 15 are not in the manual's table
        #[test]
        fn functions_04_and_15_are_refused_because_the_manual_does_not_list_them() {
            let map = map();
            let rig = Rig::new(&map, &[], "access=read-write");
            let connection = rig.connect();
            let response = rig
                .read(&*connection, vec![rig.tag("pv", "input-register:4096:INT")])
                .expect("the read resolves");
            let error = response
                .items()
                .first()
                .and_then(|item| item.outcome().as_ref().err());
            assert_eq!(
                error.map(|error| error.status().map(ProtocolStatus::code)),
                Some(Some(1)),
                "an input register read (function 04) is refused with exception 01 (assumed)"
            );
            // Stonecrab writes a coil with function 15; the manual lists function 05 only.
            let tag = rig.tag("run_stop", "coil:2068");
            let response = rig
                .write(&*connection, vec![(tag, PlcValue::Bool(false))])
                .expect("the write resolves");
            let result = response
                .get(&TagId::new("run_stop").expect("id"))
                .expect("a result");
            assert!(
                matches!(result.outcome(), WriteOutcome::Rejected(error)
                    if error.status().map(ProtocolStatus::code) == Some(1)),
                "a coil write (function 15) is refused with exception 01 (assumed)"
            );
            assert_eq!(rig.device.applied(), 0, "nothing was written");
            let mut peer = rig.peer();
            assert_eq!(
                peer(&[0x05, 0x08, 0x14, 0xFF, 0x00]),
                Some(refusal(5, 1)),
                "function 05 is listed by the manual but the responder serves no function 05 (S45)"
            );
        }

        // covers: MBS-DEVICE-SLM1C-MAP / the profile is the map's profile
        #[test]
        fn the_profile_is_documented_and_validates_for_the_rtu_driver() {
            let profile = profile(PROFILE);
            let map = map();
            check_profile(&profile, &map, FILE, 2);
            assert_eq!(profile.connection.scheme, "modbus-rtu", "the serial driver");
            let serial = map
                .device
                .serial
                .as_ref()
                .expect("the SOLO's line is documented");
            let mut options = BTreeMap::from([
                (
                    "default-unit-identifier".to_owned(),
                    map.device.unit_id.to_string(),
                ),
                ("max-registers-per-request".to_owned(), "8".to_owned()),
            ]);
            options.extend(serial.options().iter().filter_map(|option| {
                option
                    .split_once('=')
                    .map(|(key, value)| (key.to_owned(), value.to_owned()))
            }));
            assert_eq!(
                profile.connection.options, options,
                "the profile records the unit, the request limit and the line the replay uses"
            );
        }

        // covers: MBS-DEVICE-SLM1C-RTU / the replay speaks RTU on the documented line
        #[test]
        fn the_replay_speaks_rtu_framing_on_the_documented_line() {
            let rig = Rig::new(&map(), &[], OPTIONS);
            let connection = rig.connect();
            let start = rig.clock.now();
            let response = rig
                .read(
                    &*connection,
                    vec![rig.tag("pv", "holding-register:4096:INT")],
                )
                .expect("the read resolves");
            assert_eq!(
                response
                    .get(&TagId::new("pv").expect("id"))
                    .and_then(|item| item.outcome().as_ref().ok()),
                Some(&PlcValue::I16(250)),
                "register 1000H reads 250 over RTU"
            );
            let sent: Vec<Vec<u8>> = rig
                .factory
                .probes()
                .iter()
                .flat_map(stonecrab::testkit::FakeTransportProbe::sent)
                .map(|bytes| bytes.to_vec())
                .collect();
            assert_eq!(
                sent,
                [rtu_frame(1, &[0x03, 0x10, 0x00, 0x00, 0x01])],
                "one frame: network address 1, function 03, register 1000H, one register, CRC"
            );
            assert_eq!(
                rig.clock.now().duration_since(start),
                Duration::from_nanos(4_010_415),
                "the request waited the crate's t3.5 for an 11 bit character at 9600 baud (about 4.01 ms)"
            );
            let request = rtu_frame(1, &[0x03, 0x10, 0x00, 0x00, 0x01]);
            assert!(
                rig.answers_frame(&request),
                "a good frame for address 1 is answered"
            );
            let mut bad_crc = request.clone();
            if let Some(last) = bad_crc.last_mut() {
                *last ^= 0xFF;
            }
            assert!(
                !rig.answers_frame(&bad_crc),
                "a frame with a wrong CRC gets silence"
            );
            assert!(
                !rig.answers_frame(&rtu_frame(2, &[0x03, 0x10, 0x00, 0x00, 0x01])),
                "a frame for network address 2 gets silence"
            );
        }

        // covers: MBS-DEVICE-SLM1C-SENTINEL / an error code is never read as a good temperature
        #[test]
        fn a_process_variable_error_code_is_never_a_good_reading() {
            let profile = profile(PROFILE);
            let map = map();
            let nominal = measure(&profile, &map, &[]);
            let pv = named(&nominal, "process_value");
            assert_eq!(
                (pv.quality, pv.value.clone(), pv.raw.clone()),
                (
                    Quality::Good,
                    Some(PlcValue::F64(25.0)),
                    Some(PlcValue::I16(250))
                ),
                "250 is 25.0 degrees C"
            );
            for sentinel in &map.sentinels {
                let measurements = measure(&profile, &map, &[(0x1000, sentinel.word)]);
                let pv = named(&measurements, "process_value");
                assert_eq!(
                    pv.raw,
                    Some(PlcValue::I16(i16::from_be_bytes(
                        sentinel.word.to_be_bytes()
                    ))),
                    "{:#06x} ({}): the raw code is kept",
                    sentinel.word,
                    sentinel.meaning
                );
                assert_eq!(
                    pv.quality,
                    Quality::Uncertain,
                    "{:#06x}: far outside -200 to 1300 degrees C, so never Good (schema 1 cannot make it Bad)",
                    sentinel.word
                );
            }
            // An assumption the manual leaves open: a negative PV is a signed 16 bit integer.
            let cold = measure(&profile, &map, &[(0x1000, 0xFF38)]);
            assert_eq!(
                named(&cold, "process_value").value,
                Some(PlcValue::F64(-20.0)),
                "0xFF38 is -200, so -20.0 degrees C if the PV is signed (assumed)"
            );
        }

        // covers: MBS-DEVICE-SLM1C-SENTINEL / a display in Fahrenheit is never read as Celsius
        #[test]
        fn a_fahrenheit_display_makes_the_process_value_bad() {
            let profile = profile(PROFILE);
            let map = map();
            // Status bits 102AH: bit 2 degrees C, bit 3 degrees F.
            let fahrenheit = measure(&profile, &map, &[(0x102A, 0x0008)]);
            assert_eq!(
                named(&fahrenheit, "process_value").quality,
                Quality::Bad,
                "degrees F"
            );
            let neither = measure(&profile, &map, &[(0x102A, 0x0000)]);
            assert_eq!(
                named(&neither, "process_value").quality,
                Quality::Bad,
                "no unit bit"
            );
        }

        // covers: MBS-DEVICE-SLM1C-WRITE / the documented write targets accept a write and
        // read back
        #[test]
        fn the_documented_write_targets_take_a_write_and_read_back() {
            // Input type 1004H (0 is thermocouple K), set point 1001H (a negative one settles the
            // encoding of a negative number), protocol 1072H (1 is RTU) and bit length 1074H
            // (0 is 8 bits): the two the unit needs for RTU. Each is restored to its default.
            assert_write_targets(
                &map(),
                &[
                    (
                        "holding-register:4100:UINT",
                        PlcValue::U16(0),
                        PlcValue::U16(12),
                        Some(0),
                    ),
                    (
                        "holding-register:4097:INT",
                        PlcValue::I16(-500),
                        PlcValue::I16(0),
                        Some(0xFE0C),
                    ),
                    (
                        "holding-register:4210:UINT",
                        PlcValue::U16(1),
                        PlcValue::U16(0),
                        Some(1),
                    ),
                    (
                        "holding-register:4212:UINT",
                        PlcValue::U16(0),
                        PlcValue::U16(1),
                        Some(0),
                    ),
                ],
            );
        }
    }
}
