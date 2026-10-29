//! What the serial carrier adds to the diagslave evidence of Modbus RTU and Modbus ASCII
//! (slice S59c, ADR-0024): the drivers' `modbus-rtu:///path` and `modbus-ascii:///path`
//! connections to diagslave 3.6 on a real pseudo-terminal pair on the host, which
//! `interop/diagslave/serial.sh` makes with socat. diagslave opens one end as its serial port
//! (`diagslave -m rtu -a 1 ...` or `-m ascii -a 1 ...`) and the driver opens the other end
//! with the line settings of the connection string. The nine tests of
//! `modbus_interop_diagslave_rtu` and of `modbus_interop_diagslave_ascii` run over the same
//! line (they take a full connection string); the two tests here are what the gateway
//! carrier cannot show: the silence the driver keeps on the line before a request, and a port
//! that is lost and then opened again.
//!
//! Every test here is `#[ignore]`d and runs only when `STONECRAB_INTEROP_DIAGSLAVE_SERIAL`
//! names the line (for example `modbus-rtu:///tmp/line/ttyB?serial.baud-rate=19200`) and
//! `STONECRAB_INTEROP_DIAGSLAVE_SERIAL_CONTROL` names `interop/diagslave/serial.sh`, which
//! the lost-port test runs to close the line and open it again. `mise run
//! interop-serial-port` starts both and runs the tests, and so does the `interop-serial-port`
//! job of `.github/workflows/ci.yml`; the scheme of the first variable picks the driver.
//! Without the variables the tests stay ignored and touch nothing.
//!
//! # What the silence is
//! RTU frames are told apart by silence on the line (Modbus over Serial Line V1.02, 2.5.1.1),
//! and the driver waits until the line has been quiet for t3.5, 3.5 character times from the
//! line settings (1.75 ms above 19200 baud), or `serial.interframe-delay`, whichever is
//! longer, before it writes a request (MBS-RTU-SILENCE). A pseudo-terminal does not throttle
//! bytes to a baud rate, so diagslave could not show it: the test reads the time the second
//! of two back-to-back requests takes at 300 baud 8N1, where t3.5 is 116.7 ms, and with a
//! `serial.interframe-delay` of 250 ms. Only lower bounds are asserted, because a loaded
//! runner may take any longer. An ASCII frame is bounded by its colon and CR LF, so only the
//! delay applies to it (MBS-ASCII-TIMING).
//!
//! # What a lost port is
//! `line-down` ends diagslave and socat, which closes both ends of the pair and removes the
//! links; the driver's next request fails `Disconnected`. `line-up` makes the pair again at
//! the same path and starts a new diagslave, whose areas are zero again, so a read that finds
//! zero where the test had written a value was answered by the new process. The request
//! after it reopens the port (the reconnect backoff may fail it `Disconnected` a few times
//! first, as in the Toxiproxy tests).
//!
//! # Time
//! The deadlines are real, 2 s for a request and 10 s for the ones that wait out a backoff.
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
    use std::process::Command;
    use std::time::{Duration, Instant};

    use stonecrab::modbus::{ModbusAsciiDriver, ModbusRtuDriver};
    use stonecrab::{Driver, ErrorKind, PlcValue};

    use crate::interop::{Peer, value_of};

    /// The environment variable that names the line.
    const VARIABLE: &str = "STONECRAB_INTEROP_DIAGSLAVE_SERIAL";
    /// The environment variable that names the script that closes and opens the line.
    const CONTROL: &str = "STONECRAB_INTEROP_DIAGSLAVE_SERIAL_CONTROL";

    /// The baud rate `serial.sh` gives the line, and the one the silence test lowers.
    const LINE_BAUD: &str = "serial.baud-rate=19200";
    const SLOW_BAUD: &str = "serial.baud-rate=300";

    /// The deadline of a request that waits out a reconnect backoff.
    const PATIENT: Duration = Duration::from_secs(10);

    /// The line of the environment, with `extra` appended to its query.
    fn line(extra: &str) -> String {
        let base = std::env::var(VARIABLE).unwrap_or_else(|_| {
            panic!(
                "{VARIABLE} is not set: these tests need the pseudo-terminal pair of \
                 interop/diagslave/serial.sh (run `mise run interop-serial-port` on Linux, or \
                 start it and set {VARIABLE}=modbus-rtu:///<dir>/ttyB?{LINE_BAUD})"
            )
        });
        assert!(
            base.starts_with("modbus-rtu:///") || base.starts_with("modbus-ascii:///"),
            "{VARIABLE} must name a serial port (modbus-rtu:///<path> or modbus-ascii:///<path>), \
             not a gateway: {base}"
        );
        format!("{base}&access=read-write&request-timeout-ms=2000{extra}")
    }

    /// Whether the line is an RTU one; the scheme of the connection string picks the driver.
    fn is_rtu(uri: &str) -> bool {
        uri.starts_with("modbus-rtu:")
    }

    /// A connection to the line `uri` names, with the driver of its scheme.
    async fn connect(uri: &str) -> Peer {
        let driver: Box<dyn Driver> = if is_rtu(uri) {
            Box::new(ModbusRtuDriver::new())
        } else {
            Box::new(ModbusAsciiDriver::new())
        };
        Peer::connect_with(
            driver,
            uri,
            "Start the line with `mise run interop-serial-port` (Linux with socat) or point the variable at a pseudo-terminal pair that diagslave serves",
        )
        .await
    }

    /// Run a step of the control script (`line-down`, `line-up`) and fail the test when it
    /// fails.
    fn control(step: &str) {
        let script = std::env::var(CONTROL).unwrap_or_else(|_| {
            panic!("{CONTROL} is not set: it names interop/diagslave/serial.sh")
        });
        let status = Command::new(&script)
            .arg(step)
            .status()
            .unwrap_or_else(|error| panic!("cannot run {script} {step}: {error}"));
        assert!(status.success(), "{script} {step} failed: {status}");
    }

    /// Write a register, read it twice and return how long the second read took and what it
    /// read: the second request meets a line that has just carried a request and its reply.
    async fn second_read(uri: &str, address: u32, value: u16) -> (Duration, Option<PlcValue>) {
        let peer = connect(uri).await;
        let text = format!("holding-register:{address}:UINT");
        let written = peer
            .write(vec![(peer.tag("x", &text), PlcValue::U16(value))])
            .await;
        assert_eq!(written.confirmed_count(), 1, "{written:?}");
        let first = peer.read(vec![peer.tag("x", &text)]).await;
        assert_eq!(value_of(&first, "x"), Some(PlcValue::U16(value)));
        let started = Instant::now();
        let second = peer.read(vec![peer.tag("x", &text)]).await;
        let elapsed = started.elapsed();
        peer.close().await;
        (elapsed, value_of(&second, "x"))
    }

    /// The least a request may take, with 20 ms of slack for the time between the last byte of
    /// the previous reply and the start of the clock.
    fn at_least(silence: Duration) -> Duration {
        silence.saturating_sub(Duration::from_millis(20))
    }

    // covers: S59c / MBS-RTU-SILENCE, MBS-CFG-SERIAL-BAUD, MBS-CFG-SERIAL-INTERFRAME,
    // MBS-ASCII-TIMING the silence before a request, kept on a real line
    #[tokio::test]
    #[ignore = "interop: set STONECRAB_INTEROP_DIAGSLAVE_SERIAL"]
    async fn interop_serial_the_silence_before_a_request_is_kept_on_the_line() {
        let uri = line("");
        // 3.5 characters of ten bits (8N1) at 300 baud: 116.7 ms. ASCII has no t3.5, so for it
        // the run shows only that a 300 baud line is opened and answered; a loaded runner may
        // take any time, so no upper bound is asserted for either framing.
        assert!(uri.contains(LINE_BAUD), "{uri}");
        let slow = uri.replace(LINE_BAUD, SLOW_BAUD);
        let (elapsed, value) = second_read(&slow, 701, 0x5A5A).await;
        assert_eq!(value, Some(PlcValue::U16(0x5A5A)));
        if is_rtu(&uri) {
            assert!(
                elapsed >= at_least(Duration::from_micros(116_667)),
                "the second request went out after {elapsed:?} at 300 baud; t3.5 is 116.7 ms"
            );
        }
        // The delay is a floor on the silence for both framings.
        let (elapsed, value) =
            second_read(&line("&serial.interframe-delay=250"), 702, 0x1357).await;
        assert_eq!(value, Some(PlcValue::U16(0x1357)));
        assert!(
            elapsed >= at_least(Duration::from_millis(250)),
            "the second request went out after {elapsed:?} with an inter-frame delay of 250 ms"
        );
    }

    // covers: S59c / MBS-TRANS-SERIAL a lost serial port is Disconnected and the next request
    // reopens it; SPEC mandatory-failure-cases / RTU serial disconnect and reopen
    #[tokio::test]
    #[ignore = "interop: set STONECRAB_INTEROP_DIAGSLAVE_SERIAL"]
    async fn interop_serial_a_lost_port_is_disconnected_and_the_next_request_reopens_it() {
        let peer = connect(&line("")).await;
        let written = peer
            .write(vec![(
                peer.tag("x", "holding-register:801:UINT"),
                PlcValue::U16(0x1234),
            )])
            .await;
        assert_eq!(written.confirmed_count(), 1, "{written:?}");
        let before = peer
            .read(vec![peer.tag("x", "holding-register:801:UINT")])
            .await;
        assert_eq!(value_of(&before, "x"), Some(PlcValue::U16(0x1234)));

        // Both ends of the pair are closed and the links are gone.
        control("line-down");
        let lost = peer
            .try_read(
                vec![peer.tag("x", "holding-register:801:UINT")],
                Duration::from_secs(2),
            )
            .await;
        assert_eq!(
            lost.map(|_| ()).map_err(|error| error.kind()),
            Err(ErrorKind::Disconnected),
            "the port went away"
        );

        // A new pair at the same path and a new diagslave, whose areas are zero.
        control("line-up");
        let mut failures = 0_usize;
        let reopened = loop {
            match peer
                .try_read(vec![peer.tag("x", "holding-register:801:UINT")], PATIENT)
                .await
            {
                Ok(response) => break response,
                Err(error) => {
                    assert_eq!(
                        error.kind(),
                        ErrorKind::Disconnected,
                        "reopening allows Disconnected only: {error}"
                    );
                    failures += 1;
                    assert!(failures <= 3, "not reopened after {failures} failures");
                    // Real time on purpose: the test drives a real pseudo-terminal and a real
                    // diagslave process, which a paused clock cannot advance.
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            }
        };
        assert_eq!(
            value_of(&reopened, "x"),
            Some(PlcValue::U16(0)),
            "answered by the new diagslave, whose registers are zero again"
        );
        // The reopened port carries writes too.
        let again = peer
            .write(vec![(
                peer.tag("x", "holding-register:801:UINT"),
                PlcValue::U16(0x4321),
            )])
            .await;
        assert_eq!(again.confirmed_count(), 1, "{again:?}");
        let after = peer
            .read(vec![peer.tag("x", "holding-register:801:UINT")])
            .await;
        assert_eq!(value_of(&after, "x"), Some(PlcValue::U16(0x4321)));
        peer.close().await;
    }
}
