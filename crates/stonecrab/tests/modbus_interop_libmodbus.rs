//! Interoperability with libmodbus 3.1.11 (slice S52a): the driver against the
//! `unit-test-server` of the libmodbus test suite, a C stack that shares no code with
//! Stonecrab or with pymodbus, in a container built from `interop/libmodbus/`.
//!
//! Every test here is `#[ignore]`d and runs only when `STONECRAB_INTEROP_LIBMODBUS` names the
//! server (for example `modbus-tcp://127.0.0.1:1502`): `mise run interop` starts the image and
//! runs them, and so does the `interop` job of `.github/workflows/ci.yml`. Without the
//! variable the tests stay ignored and nothing here touches the network.
//!
//! # What the server is
//! The server accepts one connection, serves it until the client closes it and exits; the
//! container's `serve.sh` starts it again, with a fresh register map. So each test opens
//! exactly one connection, what a test writes is gone when it closes, and the tests run one
//! at a time (`interop.sh` passes `--test-threads 1`). The server answers any unit id by
//! echoing it, except in the byte-at-a-time reply, which always carries unit 255; the
//! tests use unit 17, the `SERVER_ID` of `unit-test.h` (the id its RTU mode listens on).
//! It never receives function code 07, which makes 3.1.11 exit without a reply.
//!
//! # Where the expected values come from
//! The constants below are transcribed from `unit-test.h` (BSD-3-Clause, Copyright
//! Stephane Raimbault, shipped in the Debian `libmodbus-dev` 3.1.11-2 package at
//! `/usr/share/doc/libmodbus-dev/examples/`) and from the behavior written in
//! `unit-test-server.c`. They are not read from the image and not the output of any decoder
//! under test. The addresses in `unit-test.h` are wire addresses; a Stonecrab address is
//! one-based, so every address below is the wire address plus one.
//!
//! The server holds only what `unit-test-server.c` assigns: the input bits (22 of them) and
//! the one input register. Coils and holding registers start at zero, which the tests check,
//! and the values `unit-test-client.c` would write first are written by the tests.
//!
//! # Time
//! The server's 500 ms delay and its byte-at-a-time reply are real time, so the deadlines
//! here are real (system clock, at most 2 s); no test sleeps.
#![cfg(all(feature = "modbus", feature = "testkit"))]
#![forbid(unsafe_code)]

#[cfg(test)]
#[path = "support/interop.rs"]
mod interop;

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use stonecrab::{ErrorKind, FailureCategory, PlcValue, ProtocolStatus, Tag};

    use crate::interop::{DEADLINE, Peer, failure, value_of};

    /// The environment variable that names the server.
    const VARIABLE: &str = "STONECRAB_INTEROP_LIBMODBUS";

    /// Connection parameters: unit 17 (`SERVER_ID`), read-only and with writes granted.
    const READ_ONLY: &str = "default-unit-identifier=17";
    const READ_WRITE: &str = "access=read-write&default-unit-identifier=17";
    /// Unit 255, which the byte-at-a-time reply carries.
    const UNIT_255: &str = "default-unit-identifier=255";

    // unit-test.h, BSD-3-Clause: wire addresses and the values the server holds or the
    // client writes.
    const UT_BITS_ADDRESS: u16 = 0x130;
    const UT_BITS_NB: u16 = 0x25;
    const UT_BITS_TAB: [u8; 5] = [0xCD, 0x6B, 0xB2, 0x0E, 0x1B];
    const UT_INPUT_BITS_ADDRESS: u16 = 0x1C4;
    const UT_INPUT_BITS_NB: u16 = 0x16;
    const UT_INPUT_BITS_TAB: [u8; 3] = [0xAC, 0xDB, 0x35];
    const UT_REGISTERS_ADDRESS: u16 = 0x160;
    const UT_REGISTERS_NB_MAX: u16 = 0x20;
    const UT_REGISTERS_TAB: [u16; 3] = [0x022B, 0x0001, 0x0064];
    /// A read of this first address is answered with exception 06.
    const UT_REGISTERS_ADDRESS_SPECIAL: u16 = 0x170;
    /// A read of this first address is answered with a wrong transaction id and unit id.
    const UT_REGISTERS_ADDRESS_INVALID_TID_OR_SLAVE: u16 = 0x171;
    /// A read of this first address is answered after 500 ms.
    const UT_REGISTERS_ADDRESS_SLEEP_500_MS: u16 = 0x172;
    /// A read of this first address is answered one byte at a time, 5 ms apart.
    const UT_REGISTERS_ADDRESS_BYTE_SLEEP_5_MS: u16 = 0x173;
    const UT_INPUT_REGISTERS_ADDRESS: u16 = 0x108;
    const UT_INPUT_REGISTERS_TAB: [u16; 1] = [0x000A];
    /// `UT_REAL` is 123456.0; the four constants are its two words in the four word orders
    /// `unit-test.h` names, in the order the register block holds them.
    const UT_REAL: f32 = 123_456.0;
    const UT_IREAL_ABCD: [u16; 2] = [0x47F1, 0x2000];
    const UT_IREAL_DCBA: [u16; 2] = [0x0020, 0xF147];
    const UT_IREAL_BADC: [u16; 2] = [0xF147, 0x0020];
    const UT_IREAL_CDAB: [u16; 2] = [0x2000, 0x47F1];

    // unit-test-server.c: a read of two holding registers (UT_REGISTERS_NB_SPECIAL) is
    // answered with the data of one.
    const UT_REGISTERS_NB_SPECIAL: u16 = 0x2;

    fn url_with(query: &str) -> String {
        let base = std::env::var(VARIABLE).unwrap_or_else(|_| {
            panic!(
                "{VARIABLE} is not set: these tests need the libmodbus server of \
                 interop/libmodbus (run `mise run interop`, or start the container and set \
                 {VARIABLE}=modbus-tcp://127.0.0.1:1502)"
            )
        });
        format!("{base}?{query}")
    }

    /// The driver connected to the server named by the variable.
    async fn connect(query: &str) -> Peer {
        let hint = format!(
            "Start it with `mise run interop` or point {VARIABLE} at a running libmodbus \
             unit-test-server"
        );
        Peer::connect(&url_with(query), &hint).await
    }

    /// The tag at the wire address `wire` of `area` (`holding-register`, `coil`, ...) read
    /// as `data_type`; Stonecrab addresses are one-based.
    fn tag_at(peer: &Peer, area: &str, wire: u16, data_type: &str) -> Tag {
        let logical = wire.saturating_add(1);
        peer.tag(
            &format!("{area}-{wire:#05x}"),
            &format!("{area}:{logical}:{data_type}"),
        )
    }

    fn register_tag(peer: &Peer, wire: u16) -> Tag {
        tag_at(peer, "holding-register", wire, "UINT")
    }

    /// The first `count` bits of a packed byte table, least significant bit first, as
    /// `unit-test.h` packs its bit tables.
    fn documented_bits(table: &[u8], count: u16) -> Vec<bool> {
        table
            .iter()
            .flat_map(|byte| (0..8_u32).map(move |index| byte & (1_u8 << index) != 0))
            .take(usize::from(count))
            .collect()
    }

    /// Read `count` bits of `area` from `wire` as one tag each, in one request.
    async fn read_bits(peer: &Peer, area: &str, wire: u16, count: u16) -> Vec<Option<PlcValue>> {
        let tags: Vec<Tag> = (0..count)
            .map(|offset| tag_at(peer, area, wire.saturating_add(offset), "BOOL"))
            .collect();
        let response = peer.read(tags).await;
        assert_eq!(
            response.split_into().get(),
            1,
            "adjacent bits are one wire request"
        );
        (0..count)
            .map(|offset| {
                let id = format!("{area}-{:#05x}", wire.saturating_add(offset));
                value_of(&response, &id)
            })
            .collect()
    }

    fn as_bools(bits: &[bool]) -> Vec<Option<PlcValue>> {
        bits.iter().map(|b| Some(PlcValue::Bool(*b))).collect()
    }

    /// Read `count` holding registers from `wire` as one UINT tag each, in one request.
    ///
    /// Never `count == 2`: the server answers every read of two holding registers with one
    /// (`UT_REGISTERS_NB_SPECIAL`), which the short-reply test below relies on.
    async fn read_registers(peer: &Peer, wire: u16, count: u16) -> Vec<Option<PlcValue>> {
        assert_ne!(count, UT_REGISTERS_NB_SPECIAL, "the server's short reply");
        let tags: Vec<Tag> = (0..count)
            .map(|offset| register_tag(peer, wire.saturating_add(offset)))
            .collect();
        let response = peer.read(tags).await;
        (0..count)
            .map(|offset| {
                let id = format!("holding-register-{:#05x}", wire.saturating_add(offset));
                value_of(&response, &id)
            })
            .collect()
    }

    fn as_words(words: &[u16]) -> Vec<Option<PlcValue>> {
        words.iter().map(|w| Some(PlcValue::U16(*w))).collect()
    }

    // covers: SPEC mandatory-failure-cases / F5 Modbus level; MB-TCP-CONNECT, MB-TCP-READ-DI,
    // MB-TCP-READ-IR, MB-CFG-UNIT-ID
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "interop: set STONECRAB_INTEROP_LIBMODBUS"]
    async fn interop_reads_the_input_bits_and_the_input_register() {
        let peer = connect(READ_ONLY).await;
        assert_eq!(
            read_bits(
                &peer,
                "discrete-input",
                UT_INPUT_BITS_ADDRESS,
                UT_INPUT_BITS_NB
            )
            .await,
            as_bools(&documented_bits(&UT_INPUT_BITS_TAB, UT_INPUT_BITS_NB)),
            "the 22 input bits 0xAC 0xDB 0x35 of unit-test.h"
        );
        let register = tag_at(&peer, "input-register", UT_INPUT_REGISTERS_ADDRESS, "UINT");
        let response = peer.read(vec![register]).await;
        assert_eq!(
            value_of(&response, "input-register-0x108"),
            Some(PlcValue::U16(UT_INPUT_REGISTERS_TAB[0])),
            "the input register 0x000A of unit-test.h"
        );
        peer.close().await;
    }

    // covers: SPEC mandatory-failure-cases / F5 Modbus level; MB-TCP-READ-COIL, MB-TCP-READ-HR
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "interop: set STONECRAB_INTEROP_LIBMODBUS"]
    async fn interop_reads_coils_and_holding_registers_at_rest() {
        let peer = connect(READ_ONLY).await;
        assert_eq!(
            read_bits(&peer, "coil", UT_BITS_ADDRESS, UT_BITS_NB).await,
            as_bools(&vec![false; usize::from(UT_BITS_NB)]),
            "the server's coils start false"
        );
        assert_eq!(
            read_registers(&peer, UT_REGISTERS_ADDRESS, UT_REGISTERS_NB_MAX).await,
            as_words(&[0; 0x20]),
            "the server's 32 holding registers start at zero"
        );
        peer.close().await;
    }

    // covers: SPEC mandatory-failure-cases / F5 Modbus level; MB-TCP-WRITE, MB-TCP-CAPABILITIES
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "interop: set STONECRAB_INTEROP_LIBMODBUS"]
    async fn interop_writes_the_unit_test_values_and_reads_them_back() {
        let peer = connect(READ_WRITE).await;

        // FC15 for each of the 37 coils (one item, one request), FC06 for each of the three
        // registers.
        let bits = documented_bits(&UT_BITS_TAB, UT_BITS_NB);
        let mut items: Vec<(Tag, PlcValue)> = Vec::new();
        for (offset, value) in (0_u16..).zip(&bits) {
            let wire = UT_BITS_ADDRESS.saturating_add(offset);
            items.push((tag_at(&peer, "coil", wire, "BOOL"), PlcValue::Bool(*value)));
        }
        for (offset, word) in (0_u16..).zip(UT_REGISTERS_TAB) {
            let wire = UT_REGISTERS_ADDRESS.saturating_add(offset);
            items.push((register_tag(&peer, wire), PlcValue::U16(word)));
        }
        let wanted = items.len();
        let written = peer.write(items).await;
        assert_eq!(written.confirmed_count(), wanted, "all echoed: {written:?}");

        assert_eq!(
            read_bits(&peer, "coil", UT_BITS_ADDRESS, UT_BITS_NB).await,
            as_bools(&bits),
            "the coils the server now holds: 0xCD 0x6B 0xB2 0x0E 0x1B"
        );
        let count = u16::try_from(UT_REGISTERS_TAB.len()).expect("three");
        assert_eq!(
            read_registers(&peer, UT_REGISTERS_ADDRESS, count).await,
            as_words(&UT_REGISTERS_TAB),
            "the registers the server now holds"
        );
        peer.close().await;
    }

    // covers: SPEC mandatory-failure-cases / F5 Modbus level; MB-TCP-WRITE, MB-BYTEORDER-BE,
    // MB-BYTEORDER-LE
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "interop: set STONECRAB_INTEROP_LIBMODBUS"]
    async fn interop_writes_a_real_in_the_four_word_orders_of_unit_test_h() {
        let peer = connect(READ_WRITE).await;
        let cases = [
            ("BIG_ENDIAN", UT_IREAL_ABCD),
            ("LITTLE_ENDIAN", UT_IREAL_DCBA),
            ("BIG_ENDIAN_BYTE_SWAP", UT_IREAL_BADC),
            ("LITTLE_ENDIAN_BYTE_SWAP", UT_IREAL_CDAB),
        ];
        for (index, (order, words)) in (0_u16..).zip(cases) {
            // FC16 writes two registers; each order gets its own four, so that the two words
            // can be read back together with the zero that follows them (a read of exactly
            // two registers is the server's short reply).
            let wire = UT_REGISTERS_ADDRESS.saturating_add(index.saturating_mul(4));
            let text = format!(
                "holding-register:{}:REAL{{byte-order: '{order}'}}",
                wire.saturating_add(1)
            );
            let written = peer
                .write(vec![(peer.tag("real", &text), PlcValue::F32(UT_REAL))])
                .await;
            assert_eq!(written.confirmed_count(), 1, "{order}: {written:?}");
            let [first, second] = words;
            assert_eq!(
                read_registers(&peer, wire, 3).await,
                as_words(&[first, second, 0]),
                "{order}: the words the server now holds"
            );
            // The same two registers as a REAL, merged with the register after them into one
            // request of three.
            let back = peer
                .read(vec![
                    peer.tag("real", &text),
                    register_tag(&peer, wire.saturating_add(2)),
                ])
                .await;
            assert_eq!(
                value_of(&back, "real"),
                Some(PlcValue::F32(UT_REAL)),
                "{order}: read back"
            );
        }
        peer.close().await;
    }

    // covers: SPEC mandatory-failure-cases / F5 Modbus level; MB-EXC-06
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "interop: set STONECRAB_INTEROP_LIBMODBUS"]
    async fn interop_exception_06_is_a_typed_device_exception_and_the_session_goes_on() {
        let peer = connect(READ_ONLY).await;
        let response = peer
            .read(vec![register_tag(&peer, UT_REGISTERS_ADDRESS_SPECIAL)])
            .await;
        let error = failure(&response, "holding-register-0x170");
        assert_eq!(error.kind(), ErrorKind::DeviceException, "exception 06");
        let status = error.status().expect("the device's status");
        assert_eq!(status.code(), 6, "the code the server sent");
        assert_eq!(status.name(), Some("SERVER DEVICE BUSY"), "its V1.1b name");

        // An exception is an answer: the connection is still good.
        let after = peer
            .read(vec![tag_at(
                &peer,
                "input-register",
                UT_INPUT_REGISTERS_ADDRESS,
                "UINT",
            )])
            .await;
        assert_eq!(
            value_of(&after, "input-register-0x108"),
            Some(PlcValue::U16(UT_INPUT_REGISTERS_TAB[0])),
            "the next read on the same connection"
        );
        peer.close().await;
    }

    // covers: SPEC mandatory-failure-cases / F5 Modbus level; MB-READ-MATCH, MB-READ-TIMEOUT,
    // MB-READ-TXN
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "interop: set STONECRAB_INTEROP_LIBMODBUS"]
    async fn interop_a_reply_with_the_wrong_transaction_and_unit_times_out_and_the_session_goes_on()
    {
        let peer = connect(READ_ONLY).await;
        let tag = register_tag(&peer, UT_REGISTERS_ADDRESS_INVALID_TID_OR_SLAVE);
        // The server answers at once, with transaction id 0 and unit 255: neither is the
        // request's, so the driver discards the reply and waits out the deadline.
        let error = peer
            .try_read(vec![tag], Duration::from_millis(300))
            .await
            .expect_err("the only reply is not for this request");
        assert_eq!(error.kind(), ErrorKind::Timeout, "no matching reply");

        // The connection is usable afterwards.
        let after = peer
            .read(vec![tag_at(
                &peer,
                "input-register",
                UT_INPUT_REGISTERS_ADDRESS,
                "UINT",
            )])
            .await;
        assert_eq!(
            value_of(&after, "input-register-0x108"),
            Some(PlcValue::U16(UT_INPUT_REGISTERS_TAB[0])),
            "the next read"
        );
        peer.close().await;
    }

    // covers: SPEC mandatory-failure-cases / F5 Modbus level; MB-READ-TIMEOUT
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "interop: set STONECRAB_INTEROP_LIBMODBUS"]
    async fn interop_a_reply_after_500_ms_is_read_within_a_2_s_deadline() {
        let peer = connect(READ_ONLY).await;
        let start = Instant::now();
        let response = peer
            .read_within(
                vec![register_tag(&peer, UT_REGISTERS_ADDRESS_SLEEP_500_MS)],
                DEADLINE,
            )
            .await;
        assert!(
            start.elapsed() >= Duration::from_millis(450),
            "the server held the reply back, so the read waited for it"
        );
        assert_eq!(
            value_of(&response, "holding-register-0x172"),
            Some(PlcValue::U16(0)),
            "the delayed reply is the register's value"
        );
        peer.close().await;
    }

    // covers: SPEC mandatory-failure-cases / F5 Modbus level; MB-READ-TIMEOUT, MB-READ-MATCH
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "interop: set STONECRAB_INTEROP_LIBMODBUS"]
    async fn interop_a_reply_after_500_ms_misses_a_200_ms_deadline_and_is_discarded_late() {
        let peer = connect(READ_ONLY).await;
        let tag = register_tag(&peer, UT_REGISTERS_ADDRESS_SLEEP_500_MS);
        let error = peer
            .try_read(vec![tag], Duration::from_millis(200))
            .await
            .expect_err("the server is still asleep at the deadline");
        assert_eq!(error.kind(), ErrorKind::Timeout, "the deadline");

        // The late reply arrives while the next request waits; it carries the earlier
        // transaction id and is discarded, and the next request gets its own answer.
        let after = peer
            .read(vec![tag_at(
                &peer,
                "input-register",
                UT_INPUT_REGISTERS_ADDRESS,
                "UINT",
            )])
            .await;
        assert_eq!(
            value_of(&after, "input-register-0x108"),
            Some(PlcValue::U16(UT_INPUT_REGISTERS_TAB[0])),
            "the next read gets its own reply, not the late one"
        );
        peer.close().await;
    }

    // covers: SPEC mandatory-failure-cases / F5 Modbus level; MB-READ-FRAMING
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "interop: set STONECRAB_INTEROP_LIBMODBUS"]
    async fn interop_a_reply_sent_a_byte_at_a_time_is_reassembled() {
        // The server writes the 11-byte reply with a pause of 5 ms before each byte, and the
        // reply carries unit 255 whatever the request's unit was.
        let peer = connect(UNIT_255).await;
        let response = peer
            .read(vec![register_tag(
                &peer,
                UT_REGISTERS_ADDRESS_BYTE_SLEEP_5_MS,
            )])
            .await;
        assert_eq!(
            value_of(&response, "holding-register-0x173"),
            Some(PlcValue::U16(0)),
            "the one register the reply holds"
        );
        peer.close().await;
    }

    // covers: SPEC mandatory-failure-cases / F5 Modbus level; MB-READ-MATCH
    // The reply decodes (one register) but answers no part of a read of two: the item fails
    // with MalformedData and the status keeps the reply PDU as the server sent it.
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "interop: set STONECRAB_INTEROP_LIBMODBUS"]
    async fn interop_a_short_reply_to_a_two_register_read_is_malformed_data() {
        let peer = connect(READ_WRITE).await;
        // A value that is not the zero of a fresh map, so the bytes asserted below can only
        // be the server's register 0x160.
        let written = peer
            .write(vec![(
                register_tag(&peer, UT_REGISTERS_ADDRESS),
                PlcValue::U16(UT_REGISTERS_TAB[0]),
            )])
            .await;
        assert_eq!(written.confirmed_count(), 1, "the register is written");

        let two_registers = tag_at(&peer, "holding-register", UT_REGISTERS_ADDRESS, "UDINT");
        let response = peer.read(vec![two_registers]).await;
        let error = failure(&response, "holding-register-0x160");
        assert_eq!(
            error.kind(),
            ErrorKind::MalformedData,
            "a reply that does not fit"
        );
        assert_eq!(
            error.error().diagnostics().category,
            Some(FailureCategory::Protocol),
            "a protocol failure, not a transport one"
        );
        // Function 03, byte count 2, the register 0x022B.
        assert_eq!(
            error.status().and_then(ProtocolStatus::raw),
            Some(&[0x03, 0x02, 0x02, 0x2B][..]),
            "the reply PDU as received"
        );

        // The frame was whole, so the stream is aligned and the connection is usable.
        let after = peer
            .read(vec![register_tag(&peer, UT_REGISTERS_ADDRESS)])
            .await;
        assert_eq!(
            value_of(&after, "holding-register-0x160"),
            Some(PlcValue::U16(UT_REGISTERS_TAB[0])),
            "the next read"
        );
        peer.close().await;
    }
}
