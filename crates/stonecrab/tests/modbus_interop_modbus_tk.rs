//! Unit-ID correlation against a scripted multi-unit server (slice S52c): the driver against
//! `interop/modbus-tk/server.py`, a first-party script on modbus-tk 1.1.5 that serves units 1,
//! 2 and 247 on one TCP port, each with a map of its own, and does not answer any other unit.
//!
//! Every test here is `#[ignore]`d and runs only when `STONECRAB_INTEROP_MODBUS_TK` names the
//! server (for example `modbus-tcp://127.0.0.1:5030`): `mise run interop` starts the image
//! and runs them, and so does the `interop-modbus-tk` job of `.github/workflows/ci.yml`.
//! Without the variable the tests stay ignored and nothing here touches the network.
//!
//! # What the server is
//! The server script is first-party, so this peer supports `interoperability-tested` at most.
//! It serves any number of connections at once and keeps them open, so the tests may run in
//! parallel; the one that writes uses scratch registers no other test reads. A request for a
//! unit it does not hold is not answered, and the connection stays open.
//!
//! # Where the expected values come from
//! The tables below are the documented maps of the header of `interop/modbus-tk/server.py`
//! and of its README, written out value by value. They are not read from the image and not the
//! output of any decoder under test. Every value differs from unit to unit wherever the maps
//! overlap, so a reply taken from the wrong unit shows in the value. The addresses here are
//! zero-based wire addresses; a Stonecrab address is one more.
//!
//! # Time
//! The unknown-unit tests wait for a deadline on the system clock (at most 300 ms each); no
//! test sleeps.
#![cfg(all(feature = "modbus", feature = "testkit"))]
#![forbid(unsafe_code)]

#[cfg(test)]
#[path = "support/interop.rs"]
mod interop;

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use stonecrab::{ErrorKind, PlcValue, ProtocolStatus, Tag};

    use crate::interop::{Peer, failure, value_of};

    /// The environment variable that names the server.
    const VARIABLE: &str = "STONECRAB_INTEROP_MODBUS_TK";

    /// Connection parameters that grant writes.
    const READ_WRITE: &str = "access=read-write";

    /// The deadline of a read whose only possible outcome is a timeout.
    const SHORT: Duration = Duration::from_millis(300);

    /// The map of one unit: the values at wire addresses 0, 1, 2 and so on, per area. An
    /// empty slice means the unit has no block of that area.
    struct UnitMap {
        unit: u8,
        holding: &'static [u16],
        input: &'static [u16],
        coils: &'static [bool],
        discrete: &'static [bool],
    }

    const T: bool = true;
    const F: bool = false;

    const UNIT_1: UnitMap = UnitMap {
        unit: 1,
        holding: &[
            1000, 1001, 1002, 1003, 1004, 1005, 1006, 1007, 1008, 1009, 1010, 1011, 1012, 1013,
            1014, 1015,
        ],
        input: &[100, 101, 102, 103],
        coils: &[T, F, T, F, T, F, T, F],
        discrete: &[T, F, F, T, F, F, T, F],
    };
    const UNIT_2: UnitMap = UnitMap {
        unit: 2,
        holding: &[
            2000, 2001, 2002, 2003, 2004, 2005, 2006, 2007, 2008, 2009, 2010, 2011, 2012, 2013,
            2014, 2015,
        ],
        input: &[200, 201, 202, 203],
        coils: &[F, T, F, T, F, T, F, T],
        discrete: &[F, T, F, F, T, F, F, T],
    };
    /// The shortest map: four holding registers, two input registers, three coils and no
    /// discrete inputs.
    const UNIT_247: UnitMap = UnitMap {
        unit: 247,
        holding: &[24700, 24701, 24702, 24703],
        input: &[2470, 2471],
        coils: &[T, T, T],
        discrete: &[],
    };
    const UNITS: [&UnitMap; 3] = [&UNIT_1, &UNIT_2, &UNIT_247];

    /// The scratch holding registers (wire 16 and 17, zero at start) of units 1 and 2, which
    /// the write test sets and puts back; unit 247 has no such registers.
    const SCRATCH: u16 = 16;

    /// A unit the server does not hold.
    const UNKNOWN: u8 = 9;

    async fn connect(query: &str) -> Peer {
        let base = std::env::var(VARIABLE).unwrap_or_else(|_| {
            panic!(
                "{VARIABLE} is not set: these tests need the modbus-tk server of \
                 interop/modbus-tk (run `mise run interop`, or start the container and set \
                 {VARIABLE}=modbus-tcp://127.0.0.1:5030)"
            )
        });
        let hint = format!(
            "Start it with `mise run interop` or point {VARIABLE} at a running \
             interop/modbus-tk server"
        );
        let url = if query.is_empty() {
            base
        } else {
            format!("{base}?{query}")
        };
        Peer::connect(&url, &hint).await
    }

    /// The id of the tag for `area` at `wire` of `unit`.
    fn id_of(unit: u8, area: &str, wire: u16) -> String {
        format!("u{unit}-{area}-{wire}")
    }

    /// The tag at the wire address `wire` of `area` on `unit`, named in its address block,
    /// read as `data_type`.
    fn tag_of(peer: &Peer, unit: u8, area: &str, wire: u16, data_type: &str) -> Tag {
        let logical = wire.saturating_add(1);
        peer.tag(
            &id_of(unit, area, wire),
            &format!("{area}:{logical}:{data_type}{{unit-id: {unit}}}"),
        )
    }

    /// One tag per address of `area` on `unit`, from wire address 0 for `count` addresses.
    fn tags_for(peer: &Peer, unit: u8, area: &str, count: usize, data_type: &str) -> Vec<Tag> {
        (0_u16..)
            .take(count)
            .map(|wire| tag_of(peer, unit, area, wire, data_type))
            .collect()
    }

    /// What the response holds for the first `count` addresses of `area` on `unit`.
    fn values_of(
        response: &stonecrab::ReadResponse,
        unit: u8,
        area: &str,
        count: usize,
    ) -> Vec<Option<PlcValue>> {
        (0_u16..)
            .take(count)
            .map(|wire| value_of(response, &id_of(unit, area, wire)))
            .collect()
    }

    fn words(values: &[u16]) -> Vec<Option<PlcValue>> {
        values.iter().map(|w| Some(PlcValue::U16(*w))).collect()
    }

    fn bits(values: &[bool]) -> Vec<Option<PlcValue>> {
        values.iter().map(|b| Some(PlcValue::Bool(*b))).collect()
    }

    // covers: SPEC mandatory-failure-cases / F5 Modbus level; MB-READ-MATCH, MB-READ-MERGE,
    // MB-ADDR-CONFIG-BLOCK, MB-TCP-READ-HR, MB-TCP-READ-IR, MB-TCP-READ-COIL, MB-TCP-READ-DI
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "interop: set STONECRAB_INTEROP_MODBUS_TK"]
    async fn interop_each_unit_on_one_connection_answers_with_its_own_map() {
        let peer = connect("").await;
        for map in UNITS {
            let unit = map.unit;
            let mut tags = tags_for(&peer, unit, "holding-register", map.holding.len(), "UINT");
            tags.extend(tags_for(
                &peer,
                unit,
                "input-register",
                map.input.len(),
                "UINT",
            ));
            tags.extend(tags_for(&peer, unit, "coil", map.coils.len(), "BOOL"));
            tags.extend(tags_for(
                &peer,
                unit,
                "discrete-input",
                map.discrete.len(),
                "BOOL",
            ));
            let response = peer.read(tags).await;
            assert_eq!(
                values_of(&response, unit, "holding-register", map.holding.len()),
                words(map.holding),
                "unit {unit}: holding registers"
            );
            assert_eq!(
                values_of(&response, unit, "input-register", map.input.len()),
                words(map.input),
                "unit {unit}: input registers"
            );
            assert_eq!(
                values_of(&response, unit, "coil", map.coils.len()),
                bits(map.coils),
                "unit {unit}: coils"
            );
            assert_eq!(
                values_of(&response, unit, "discrete-input", map.discrete.len()),
                bits(map.discrete),
                "unit {unit}: discrete inputs"
            );
        }
        peer.close().await;
    }

    // covers: SPEC mandatory-failure-cases / F5 Modbus level; MB-READ-MATCH, MB-READ-MERGE
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "interop: set STONECRAB_INTEROP_MODBUS_TK"]
    async fn interop_one_batch_across_units_gets_each_tag_its_own_units_value() {
        let peer = connect("").await;
        // The same two wire addresses on three units, in an order that interleaves the units.
        let mut tags = Vec::new();
        for wire in [0_u16, 1] {
            for map in UNITS {
                tags.push(tag_of(&peer, map.unit, "holding-register", wire, "UINT"));
            }
        }
        let response = peer.read(tags).await;
        for map in UNITS {
            assert_eq!(
                values_of(&response, map.unit, "holding-register", 2),
                words(map.holding.get(..2).unwrap_or_default()),
                "unit {}: its own first two registers",
                map.unit
            );
        }
        assert_eq!(
            response.split_into().get(),
            3,
            "one request per unit: tags of different units are never merged"
        );
        peer.close().await;
    }

    // covers: SPEC mandatory-failure-cases / F5 Modbus level; MB-READ-MATCH, MB-READ-TXN
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "interop: set STONECRAB_INTEROP_MODBUS_TK"]
    async fn interop_requests_to_alternating_units_and_functions_get_the_right_answers() {
        let peer = connect("").await;
        // Single-tag reads in sequence, each with a transaction id of its own, the units and
        // function codes (03, 04, 01) interleaved.
        let order = [
            (&UNIT_1, "holding-register"),
            (&UNIT_2, "holding-register"),
            (&UNIT_247, "input-register"),
            (&UNIT_2, "coil"),
            (&UNIT_1, "input-register"),
            (&UNIT_247, "holding-register"),
            (&UNIT_1, "coil"),
            (&UNIT_2, "input-register"),
            (&UNIT_247, "coil"),
            (&UNIT_2, "holding-register"),
        ];
        for (map, area) in order {
            let data_type = if area == "coil" { "BOOL" } else { "UINT" };
            let response = peer
                .read(vec![tag_of(&peer, map.unit, area, 1, data_type)])
                .await;
            let expected = match area {
                "holding-register" => map.holding.get(1).map(|word| PlcValue::U16(*word)),
                "input-register" => map.input.get(1).map(|word| PlcValue::U16(*word)),
                _ => map.coils.get(1).map(|bit| PlcValue::Bool(*bit)),
            };
            assert!(expected.is_some(), "the documented map has the address");
            assert_eq!(
                value_of(&response, &id_of(map.unit, area, 1)),
                expected,
                "unit {} {area} at wire 1",
                map.unit
            );
        }
        peer.close().await;
    }

    // covers: SPEC mandatory-failure-cases / F5 Modbus level; MB-EXC-02, MB-READ-MATCH,
    // MB-READ-MERGE
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "interop: set STONECRAB_INTEROP_MODBUS_TK"]
    async fn interop_a_unit_with_a_shorter_map_refuses_an_address_the_others_serve() {
        let peer = connect("").await;
        // Holding register at wire 8: units 1 and 2 hold it, unit 247 has four registers.
        let response = peer
            .read(
                UNITS
                    .iter()
                    .map(|map| tag_of(&peer, map.unit, "holding-register", 8, "UINT"))
                    .collect(),
            )
            .await;
        assert_eq!(
            value_of(&response, &id_of(1, "holding-register", 8)),
            Some(PlcValue::U16(1008)),
            "unit 1"
        );
        assert_eq!(
            value_of(&response, &id_of(2, "holding-register", 8)),
            Some(PlcValue::U16(2008)),
            "unit 2"
        );
        let error = failure(&response, &id_of(247, "holding-register", 8));
        assert_eq!(error.kind(), ErrorKind::InvalidAddress, "exception 02");
        assert_eq!(
            error.status().map(ProtocolStatus::code),
            Some(2),
            "the device's code is ILLEGAL DATA ADDRESS"
        );

        // The refusal is an answer of unit 247's: its own map still reads.
        let after = peer
            .read(vec![tag_of(&peer, 247, "holding-register", 3, "UINT")])
            .await;
        assert_eq!(
            value_of(&after, &id_of(247, "holding-register", 3)),
            Some(PlcValue::U16(24703)),
            "the next read on the connection"
        );
        peer.close().await;
    }

    // covers: SPEC mandatory-failure-cases / F5 Modbus level; MB-READ-TIMEOUT, MB-READ-MATCH
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "interop: set STONECRAB_INTEROP_MODBUS_TK"]
    async fn interop_an_unknown_unit_times_out_at_the_deadline_and_the_connection_goes_on() {
        let peer = connect("").await;
        // 255 is a valid unit id the server does not hold, like 9.
        for unit in [UNKNOWN, 255] {
            let start = Instant::now();
            let error = peer
                .try_read(
                    vec![tag_of(&peer, unit, "holding-register", 0, "UINT")],
                    SHORT,
                )
                .await
                .expect_err("no reply is ever sent for an unknown unit");
            let elapsed = start.elapsed();
            assert_eq!(error.kind(), ErrorKind::Timeout, "unit {unit}: no reply");
            assert!(
                elapsed >= Duration::from_millis(250),
                "unit {unit}: the read waited for its deadline, not less ({elapsed:?})"
            );
            assert!(
                elapsed < Duration::from_secs(2),
                "unit {unit}: and ended near it ({elapsed:?})"
            );

            // Nothing was lost on the connection: the next request gets its own reply.
            let after = peer
                .read(vec![tag_of(&peer, 2, "holding-register", 0, "UINT")])
                .await;
            assert_eq!(
                value_of(&after, &id_of(2, "holding-register", 0)),
                Some(PlcValue::U16(2000)),
                "after unit {unit}: the next read"
            );
        }
        peer.close().await;
    }

    // covers: SPEC mandatory-failure-cases / F5 Modbus level; MB-READ-TIMEOUT, MB-READ-MERGE
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "interop: set STONECRAB_INTEROP_MODBUS_TK"]
    async fn interop_a_batch_with_an_unknown_unit_ends_in_the_timeout() {
        let peer = connect("").await;
        let error = peer
            .try_read(
                vec![
                    tag_of(&peer, 1, "holding-register", 0, "UINT"),
                    tag_of(&peer, UNKNOWN, "holding-register", 0, "UINT"),
                ],
                SHORT,
            )
            .await
            .expect_err("the unknown unit's request is never answered");
        assert_eq!(error.kind(), ErrorKind::Timeout, "the whole read");

        let after = peer
            .read(vec![tag_of(&peer, 1, "holding-register", 0, "UINT")])
            .await;
        assert_eq!(
            value_of(&after, &id_of(1, "holding-register", 0)),
            Some(PlcValue::U16(1000)),
            "the next read"
        );
        peer.close().await;
    }

    // covers: SPEC mandatory-failure-cases / F5 Modbus level; MB-CFG-UNIT-ID,
    // MB-ADDR-CONFIG-BLOCK, MB-READ-MATCH
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "interop: set STONECRAB_INTEROP_MODBUS_TK"]
    async fn interop_the_unit_in_the_address_block_overrides_the_default_unit_identifier() {
        // Default unit 2: a tag with no unit in its address goes to unit 2, one with a unit
        // goes there.
        let peer = connect("default-unit-identifier=2").await;
        let response = peer
            .read(vec![
                peer.tag("default", "holding-register:1:UINT"),
                tag_of(&peer, 1, "holding-register", 0, "UINT"),
                tag_of(&peer, 247, "holding-register", 0, "UINT"),
            ])
            .await;
        assert_eq!(
            value_of(&response, "default"),
            Some(PlcValue::U16(2000)),
            "no unit in the address: the connection's default, unit 2"
        );
        assert_eq!(
            value_of(&response, &id_of(1, "holding-register", 0)),
            Some(PlcValue::U16(1000)),
            "unit 1 named in the address block"
        );
        assert_eq!(
            value_of(&response, &id_of(247, "holding-register", 0)),
            Some(PlcValue::U16(24700)),
            "unit 247 named in the address block"
        );
        assert_eq!(response.split_into().get(), 3, "one request per unit");
        peer.close().await;

        // Default unit 9, which the server does not hold: the default is silent, a unit in
        // the address is not, on the same connection.
        let peer = connect("default-unit-identifier=9").await;
        let error = peer
            .try_read(vec![peer.tag("default", "holding-register:1:UINT")], SHORT)
            .await
            .expect_err("the default unit is not served");
        assert_eq!(error.kind(), ErrorKind::Timeout, "the default unit 9");
        let named = peer
            .read(vec![tag_of(&peer, 247, "holding-register", 2, "UINT")])
            .await;
        assert_eq!(
            value_of(&named, &id_of(247, "holding-register", 2)),
            Some(PlcValue::U16(24702)),
            "the address block's unit wins over the unserved default"
        );
        peer.close().await;
    }

    // covers: SPEC mandatory-failure-cases / F5 Modbus level; MB-TCP-WRITE, MB-READ-MATCH,
    // MB-EXC-02
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "interop: set STONECRAB_INTEROP_MODBUS_TK"]
    async fn interop_a_write_to_one_unit_changes_only_that_units_map() {
        let peer = connect(READ_WRITE).await;
        let scratch = |unit: u8, offset: u16| {
            tag_of(
                &peer,
                unit,
                "holding-register",
                SCRATCH.saturating_add(offset),
                "UINT",
            )
        };
        // The same address on three units; unit 247 has no such register.
        let written = peer
            .write(vec![
                (scratch(1, 0), PlcValue::U16(0x1111)),
                (scratch(2, 0), PlcValue::U16(0x2222)),
                (scratch(247, 0), PlcValue::U16(0x7777)),
            ])
            .await;
        assert_eq!(written.confirmed_count(), 2, "units 1 and 2: {written:?}");
        assert_eq!(written.rejected_count(), 1, "unit 247: {written:?}");

        let response = peer
            .read(vec![
                scratch(1, 0),
                scratch(1, 1),
                scratch(2, 0),
                scratch(2, 1),
            ])
            .await;
        assert_eq!(
            value_of(&response, &id_of(1, "holding-register", SCRATCH)),
            Some(PlcValue::U16(0x1111)),
            "unit 1 holds its own write"
        );
        assert_eq!(
            value_of(&response, &id_of(2, "holding-register", SCRATCH)),
            Some(PlcValue::U16(0x2222)),
            "unit 2 holds its own write"
        );
        for unit in [1, 2] {
            assert_eq!(
                value_of(
                    &response,
                    &id_of(unit, "holding-register", SCRATCH.saturating_add(1))
                ),
                Some(PlcValue::U16(0)),
                "unit {unit}: the register next to the written one is untouched"
            );
        }

        // Put the scratch registers back.
        let restored = peer
            .write(vec![
                (scratch(1, 0), PlcValue::U16(0)),
                (scratch(2, 0), PlcValue::U16(0)),
            ])
            .await;
        assert_eq!(restored.confirmed_count(), 2, "restored: {restored:?}");
        peer.close().await;
    }
}
