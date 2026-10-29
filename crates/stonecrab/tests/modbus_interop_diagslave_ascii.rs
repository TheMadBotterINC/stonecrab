//! Interoperability of Modbus ASCII framing and the LRC (slice S60b, ADR-0024): the
//! `modbus-ascii` driver over its `tcp` carrier, `modbus-ascii:tcp://host:port`, against
//! diagslave 3.6, the free Modbus slave simulator of proconX, run as `diagslave -m ascii -a 1
//! -b 19200 -d 7 -s 1 -p even` on a serial device, in a container built from
//! `interop/diagslave/`. The device is one end of a pseudo-terminal pair that socat makes; the
//! other end is bridged to a TCP port, so the ASCII frames of the driver reach the framing
//! diagslave uses on a serial line (it reads the colon, the hex digits and the CR LF, and
//! checks the LRC of every request). diagslave shares no code with Stonecrab: a driver that
//! encoded a request wrongly or computed the LRC wrongly would get no reply or an exception,
//! and one that mis-framed or mis-checked a reply would read a wrong value.
//!
//! Every test here is `#[ignore]`d and runs only when `STONECRAB_INTEROP_DIAGSLAVE_ASCII` names
//! the server (for example `modbus-ascii:tcp://127.0.0.1:1504`): `mise run interop-ascii` (and
//! `mise run interop`) starts the container and runs them, and so does the `interop-ascii` job
//! of `.github/workflows/ci.yml`. Without the variable the tests stay ignored and nothing here
//! touches the network.
//!
//! # What the server is
//! diagslave 3.6 keeps its areas in memory, zero until written, and never forgets a write
//! between connections. Its input registers are its holding registers and its discrete
//! inputs are its coils (a write to one is read from the other), so the input areas are
//! read here at the cells the test wrote through the holding and coil areas. It serves unit
//! 1 only (`-a 1`; without `-a` it answers every unit). On this serial line a frame for
//! another unit gets no reply at all, so the driver's answer is `Timeout` (diagslave closes
//! the connection instead in its `-m enc` mode, see `modbus_interop_diagslave_rtu`). It
//! answers every read (function code 20) and write (21) of a file record with exception 02
//! (illegal data address), whatever the file: the extended registers of Stonecrab are such
//! records. Its log names the operation of a request ("readHoldingRegisters", "writeCoils")
//! with the first reference (one-based, like Stonecrab's addresses) and the count, but it
//! does not tell function code 05 from 15 or 06 from 16, so which codes the driver sends is
//! pinned by `tests/modbus_ascii.rs`, not here. Each test writes the values it reads, in
//! cells of its own, with the values chosen here; nothing is read from the image. The byte
//! order inside a register is not asserted here (a value is read back as it was written).
//!
//! # One master
//! The bridge serves one TCP connection at a time, like a serial line has one master: the
//! next connection waits until the previous one is closed. Each test therefore holds one
//! connection, and the test of the unit the slave does not serve closes one before it opens
//! the next.
//!
//! # Time
//! The deadline of every call is real (the system clock) and at most 2 s; no test sleeps.
#![cfg(all(feature = "modbus", feature = "testkit"))]
#![forbid(unsafe_code)]

#[cfg(test)]
#[path = "support/interop.rs"]
#[expect(
    dead_code,
    reason = "the module is shared with the other interop files; these tests need only part of it"
)]
mod interop;

#[cfg(test)]
mod tests {
    use stonecrab::modbus::ModbusAsciiDriver;
    use stonecrab::{
        ErrorKind, PlcValue, ProtocolStatus, WriteOutcome, WriteResponse, WriteResult,
    };

    use crate::interop::{Peer, failure, value_of};

    /// The environment variable that names the server.
    const VARIABLE: &str = "STONECRAB_INTEROP_DIAGSLAVE_ASCII";

    /// A connection to the server with writes granted; `extra` is appended to the query.
    async fn connect(extra: &str) -> Peer {
        let base = std::env::var(VARIABLE).unwrap_or_else(|_| {
            panic!(
                "{VARIABLE} is not set: these tests need the diagslave container of \
                 interop/diagslave (run `mise run interop-ascii`, or start the container and set \
                 {VARIABLE}=modbus-ascii:tcp://127.0.0.1:1504)"
            )
        });
        Peer::connect_with(
            Box::new(ModbusAsciiDriver::new()),
            &format!("{base}?access=read-write&request-timeout-ms=2000{extra}"),
            "Start it with `mise run interop-ascii` or point the variable at a diagslave -m ascii -a 1",
        )
        .await
    }

    /// Whether every item of the write was confirmed by the slave.
    fn all_confirmed(written: &WriteResponse) -> bool {
        written
            .results()
            .iter()
            .all(|result| matches!(result.outcome(), WriteOutcome::Confirmed))
    }

    // covers: S60b / MBS-OP-READ, MBS-OP-WRITE, MBS-ASCII-SEND, MBS-ASCII-LRC holding registers against an independent slave
    #[tokio::test]
    #[ignore = "interop: set STONECRAB_INTEROP_DIAGSLAVE_ASCII"]
    async fn interop_ascii_holding_registers_are_written_and_read_back() {
        let peer = connect("").await;
        let written = peer
            .write(vec![
                (
                    peer.tag("one", "holding-register:101:UINT"),
                    PlcValue::U16(0xBEEF),
                ),
                (
                    peer.tag("two", "holding-register:102:UDINT"),
                    PlcValue::U32(0x0102_0304),
                ),
                (
                    peer.tag("real", "holding-register:104:REAL"),
                    PlcValue::F32(-1234.5),
                ),
            ])
            .await;
        assert!(all_confirmed(&written), "{written:?}");
        let response = peer
            .read(vec![
                peer.tag("one", "holding-register:101:UINT"),
                peer.tag("two", "holding-register:102:UDINT"),
                peer.tag("real", "holding-register:104:REAL"),
                peer.tag("high", "holding-register:102:UINT"),
                peer.tag("low", "holding-register:103:UINT"),
            ])
            .await;
        assert_eq!(value_of(&response, "one"), Some(PlcValue::U16(0xBEEF)));
        assert_eq!(value_of(&response, "two"), Some(PlcValue::U32(0x0102_0304)));
        assert_eq!(value_of(&response, "real"), Some(PlcValue::F32(-1234.5)));
        assert_eq!(value_of(&response, "high"), Some(PlcValue::U16(0x0102)));
        assert_eq!(value_of(&response, "low"), Some(PlcValue::U16(0x0304)));
        peer.close().await;
    }

    // covers: S60b / MBS-OP-READ input registers (diagslave's are its holding registers)
    #[tokio::test]
    #[ignore = "interop: set STONECRAB_INTEROP_DIAGSLAVE_ASCII"]
    async fn interop_ascii_input_registers_read_what_diagslave_aliases_to_the_holding_registers() {
        let peer = connect("").await;
        let written = peer
            .write(vec![
                (
                    peer.tag("a", "holding-register:201:UINT"),
                    PlcValue::U16(0x1357),
                ),
                (
                    peer.tag("b", "holding-register:202:UINT"),
                    PlcValue::U16(0x9BDF),
                ),
            ])
            .await;
        assert!(all_confirmed(&written), "{written:?}");
        let response = peer
            .read(vec![
                peer.tag("a", "input-register:201:UINT"),
                peer.tag("b", "input-register:202:INT"),
            ])
            .await;
        assert_eq!(value_of(&response, "a"), Some(PlcValue::U16(0x1357)));
        assert_eq!(value_of(&response, "b"), Some(PlcValue::I16(-25_633)));
        peer.close().await;
    }

    // covers: S60b / MBS-OP-READ, MBS-OP-WRITE coils against an independent slave
    #[tokio::test]
    #[ignore = "interop: set STONECRAB_INTEROP_DIAGSLAVE_ASCII"]
    async fn interop_ascii_coils_are_written_and_read_back() {
        let peer = connect("").await;
        let pattern = [
            true, false, true, true, false, false, true, false, true, true,
        ];
        let items = pattern
            .iter()
            .enumerate()
            .map(|(index, bit)| {
                (
                    peer.tag(&format!("c{index}"), &format!("coil:{}", 301 + index)),
                    PlcValue::Bool(*bit),
                )
            })
            .collect::<Vec<_>>();
        // One request per item, as for Modbus TCP: ten confirmations, ten coils.
        let written = peer.write(items).await;
        assert_eq!(written.confirmed_count(), pattern.len(), "{written:?}");
        let tags = (0..pattern.len())
            .map(|index| peer.tag(&format!("c{index}"), &format!("coil:{}", 301 + index)))
            .collect();
        let response = peer.read(tags).await;
        for (index, bit) in pattern.iter().enumerate() {
            assert_eq!(
                value_of(&response, &format!("c{index}")),
                Some(PlcValue::Bool(*bit)),
                "coil {}",
                301 + index
            );
        }
        peer.close().await;
    }

    // covers: S60b / MBS-OP-READ discrete inputs (diagslave's are its coils)
    #[tokio::test]
    #[ignore = "interop: set STONECRAB_INTEROP_DIAGSLAVE_ASCII"]
    async fn interop_ascii_discrete_inputs_read_what_diagslave_aliases_to_the_coils() {
        let peer = connect("").await;
        let pattern = [false, true, true, false, true];
        let items = pattern
            .iter()
            .enumerate()
            .map(|(index, bit)| {
                (
                    peer.tag(&format!("c{index}"), &format!("coil:{}", 401 + index)),
                    PlcValue::Bool(*bit),
                )
            })
            .collect::<Vec<_>>();
        let written = peer.write(items).await;
        assert_eq!(written.confirmed_count(), pattern.len(), "{written:?}");
        let tags = (0..pattern.len())
            .map(|index| {
                peer.tag(
                    &format!("d{index}"),
                    &format!("discrete-input:{}", 401 + index),
                )
            })
            .collect();
        let response = peer.read(tags).await;
        for (index, bit) in pattern.iter().enumerate() {
            assert_eq!(
                value_of(&response, &format!("d{index}")),
                Some(PlcValue::Bool(*bit)),
                "discrete input {}",
                401 + index
            );
        }
        peer.close().await;
    }

    // covers: S60b / MBS-OP-READ a range selection is one request and one list
    #[tokio::test]
    #[ignore = "interop: set STONECRAB_INTEROP_DIAGSLAVE_ASCII"]
    async fn interop_ascii_a_range_selection_is_one_request_and_one_list_of_what_was_written() {
        let peer = connect("").await;
        let values = [0x0A0B_u16, 0x0C0D, 0x0E0F, 0xFFFF, 0x0000, 0x8000];
        let written = peer
            .write(vec![(
                peer.tag("range", "holding-register:501[0..5]:UINT"),
                PlcValue::List(values.iter().map(|value| PlcValue::U16(*value)).collect()),
            )])
            .await;
        assert!(all_confirmed(&written), "{written:?}");
        let response = peer
            .read(vec![peer.tag("range", "holding-register:501[0..5]:UINT")])
            .await;
        assert_eq!(
            value_of(&response, "range"),
            Some(PlcValue::List(
                values.iter().map(|value| PlcValue::U16(*value)).collect()
            ))
        );
        assert_eq!(response.split_into().get(), 1, "one request");
        peer.close().await;
    }

    // covers: S60b / MBS-LIMIT-ADU, MBS-LIMIT-ASCII-BUFFER, MBS-ASCII-DELIMITERS the longest frames each way (511 characters)
    #[tokio::test]
    #[ignore = "interop: set STONECRAB_INTEROP_DIAGSLAVE_ASCII"]
    async fn interop_ascii_the_longest_frames_are_framed_and_checked_each_way() {
        let peer = connect("").await;
        let word = |index: u16| index.wrapping_mul(0x0101).wrapping_add(0x1357);
        let list = |from: u16, to: u16| {
            PlcValue::List((from..to).map(|index| PlcValue::U16(word(index))).collect())
        };
        // 123 registers is the most one write carries: a request of 511 characters
        // (the colon, 508 hex digits of 254 bytes and CR LF).
        let written = peer
            .write(vec![
                (
                    peer.tag("body", "holding-register:1001[0..122]:UINT"),
                    list(0, 123),
                ),
                (
                    peer.tag("tail", "holding-register:1124[0..1]:UINT"),
                    list(123, 125),
                ),
            ])
            .await;
        assert!(all_confirmed(&written), "{written:?}");
        // 125 registers is the most one read returns: a reply of 511 characters.
        let response = peer
            .read(vec![peer.tag("all", "holding-register:1001[0..124]:UINT")])
            .await;
        assert_eq!(value_of(&response, "all"), Some(list(0, 125)));
        assert_eq!(
            response.split_into().get(),
            1,
            "one request of 125 registers"
        );
        peer.close().await;
    }

    // covers: SPEC mandatory-failure-cases / F5 Modbus level; S60b / MBS-BUS-EXCEPTION
    #[tokio::test]
    #[ignore = "interop: set STONECRAB_INTEROP_DIAGSLAVE_ASCII"]
    async fn interop_ascii_exception_02_is_a_typed_device_exception_and_the_session_goes_on() {
        let peer = connect("").await;
        let seeded = peer
            .write(vec![(
                peer.tag("ok", "holding-register:601:UINT"),
                PlcValue::U16(0x1357),
            )])
            .await;
        assert!(all_confirmed(&seeded), "{seeded:?}");
        // Function code 20 (read file record), which Stonecrab sends for an extended register.
        let response = peer
            .read(vec![
                peer.tag("file", "extended-register:1:UINT"),
                peer.tag("ok", "holding-register:601:UINT"),
            ])
            .await;
        let error = failure(&response, "file");
        assert_eq!(error.kind(), ErrorKind::InvalidAddress, "exception 02");
        assert_eq!(error.status().map(ProtocolStatus::code), Some(2));
        assert_eq!(
            value_of(&response, "ok"),
            Some(PlcValue::U16(0x1357)),
            "the other item is read"
        );
        // Function code 21 (write file record).
        let written = peer
            .write(vec![(
                peer.tag("file", "extended-register:1:UINT"),
                PlcValue::U16(7),
            )])
            .await;
        let WriteOutcome::Rejected(rejected) = written
            .results()
            .first()
            .map(WriteResult::outcome)
            .expect("a result")
        else {
            panic!("the write was expected to be rejected: {written:?}");
        };
        assert_eq!(rejected.kind(), ErrorKind::InvalidAddress, "exception 02");
        // An exception is an answer: the connection is still good.
        let after = peer
            .write(vec![(
                peer.tag("ok", "holding-register:601:UINT"),
                PlcValue::U16(0x2468),
            )])
            .await;
        assert!(all_confirmed(&after), "{after:?}");
        let again = peer
            .read(vec![peer.tag("ok", "holding-register:601:UINT")])
            .await;
        assert_eq!(value_of(&again, "ok"), Some(PlcValue::U16(0x2468)));
        peer.close().await;
    }

    // covers: S60b / MBS-ADDR-UNIT-ID, MBS-BUS-CORRELATION a unit the slave does not serve
    #[tokio::test]
    #[ignore = "interop: set STONECRAB_INTEROP_DIAGSLAVE_ASCII"]
    async fn interop_ascii_a_unit_the_slave_does_not_serve_is_silence() {
        let served = connect("").await;
        let seeded = served
            .write(vec![(
                served.tag("x", "holding-register:101:UINT"),
                PlcValue::U16(0x4321),
            )])
            .await;
        assert!(all_confirmed(&seeded), "{seeded:?}");
        // One master on the line: the first connection is closed before the next is opened.
        served.close().await;
        let other = connect("&default-unit-identifier=2").await;
        let outcome = other
            .try_read(
                vec![other.tag("x", "holding-register:101:UINT")],
                std::time::Duration::from_secs(2),
            )
            .await;
        // diagslave answers a frame for a unit it does not serve with nothing at all.
        assert_eq!(
            outcome.map(|_| ()).map_err(|error| error.kind()),
            Err(ErrorKind::Timeout),
            "unit 2"
        );
        other.close().await;
        // The slave is still there for unit 1.
        let again = connect("").await;
        let response = again
            .read(vec![again.tag("x", "holding-register:101:UINT")])
            .await;
        assert_eq!(
            value_of(&response, "x"),
            Some(PlcValue::U16(0x4321)),
            "unit 1 after the silence"
        );
        again.close().await;
    }

    // covers: S60b / MBS-OP-PING
    #[tokio::test]
    #[ignore = "interop: set STONECRAB_INTEROP_DIAGSLAVE_ASCII"]
    async fn interop_ascii_ping_reads_the_ping_address() {
        let peer = connect("").await;
        peer.connection
            .ping()
            .await
            .expect("ping reads the ping address");
        peer.close().await;
    }
}
