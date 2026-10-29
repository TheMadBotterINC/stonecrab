//! Failure paths on a real clock (slice S52b): the driver against pymodbus 3.8.6 and
//! libmodbus 3.1.11 with [Toxiproxy] 2.13.0 between them, and against a peer that is paused
//! or killed with `docker pause` and `docker kill`.
//!
//! Every test here is `#[ignore]`d and runs only when `STONECRAB_INTEROP_TOXIPROXY` names
//! the Toxiproxy API (for example `127.0.0.1:18474`) and the two variables below it name the
//! proxied peers: `mise run interop-toxiproxy` starts the containers of `interop/toxiproxy/`
//! and runs them, and so does the `interop-toxiproxy` job of `.github/workflows/ci.yml`.
//! Without the variables the tests stay ignored and nothing here touches the network.
//!
//! # How the faults are made
//! The test process is the controller: it adds and removes toxics over the Toxiproxy HTTP API
//! (a small client on `std::net`, no new dependency), and removes them again after a real
//! pause that it times itself, which a script around separate test runs could not do. The API
//! is published on the loopback of the host only. The tests run one at a time
//! (`--test-threads 1`), because a toxic belongs to the proxy and not to a connection, and
//! libmodbus serves one connection at a time. Each test begins and ends with the API's
//! `/reset`, which removes every toxic and enables every proxy.
//!
//! # Time
//! Unlike the other interop files, some of these tests wait on purpose: a deadline is a
//! real 400 to 600 ms (the system clock the driver uses), a latency toxic delays by real
//! milliseconds, and the recovery after a restart polls with a real 200 ms pause. The
//! bounds asserted are: a timeout is not earlier than its deadline and not later than the
//! deadline plus a second of slack; a request that must answer through 300 ms of latency
//! takes at least 200 ms; sliced frames take at least 10 ms; a `Disconnected` outcome
//! arrives within an upper bound; and the recovery loops, including the one after a kill,
//! give up after 60 s. Nothing else is timed.
//!
//! # What the outcomes mean
//! A timeout leaves the connection `Connected` (the late reply is discarded), so the next
//! request reuses it. A peer that closes or resets, or a proxy that is disabled, is
//! `Disconnected`, and the request after it waits out the backoff and reopens. A toxic that
//! the proxy removes by closing the link (a `timeout` toxic closes it when it is removed) is
//! noticed by the request after it, which is `Disconnected` once, so recovery is allowed a few
//! such failures and no other kind.
//!
//! [Toxiproxy]: https://github.com/Shopify/toxiproxy
#![cfg(all(feature = "modbus", feature = "testkit"))]
#![forbid(unsafe_code)]

#[cfg(test)]
#[path = "support/interop.rs"]
#[expect(
    dead_code,
    reason = "the module is shared with the other interop files; these tests read one tag at a time and write nothing"
)]
mod interop;

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::process::Command;
    use std::time::{Duration, Instant};

    use stonecrab::{ErrorKind, PlcError, PlcValue, ReadResponse, Tag};

    use crate::interop::{Peer, value_of};

    /// The environment variable that names the Toxiproxy API, `host:port`.
    const API: &str = "STONECRAB_INTEROP_TOXIPROXY";

    /// A peer behind a proxy: where its connection string comes from, what to read from it
    /// and what that read must return.
    struct Target {
        /// The name of the proxy in `interop/toxiproxy/toxiproxy.json`.
        name: &'static str,
        /// The environment variable that holds the proxied `modbus-tcp://` URL.
        variable: &'static str,
        /// The query of the connection string.
        query: &'static str,
        /// The tag every test reads: one register the peer documents.
        address: &'static str,
        /// The value of that register.
        expected: u16,
        /// A second documented register, other than `address`, for the tests that must tell
        /// a stale reply from the right one by its value.
        other_address: &'static str,
        /// The value of that register.
        other_expected: u16,
        /// The first (one-based) of ten adjacent holding registers, for the sliced read.
        block: u16,
        /// The container, for the tests that pause or kill the peer.
        container: &'static str,
    }

    const PYMODBUS: Target = Target {
        name: "pymodbus",
        variable: "STONECRAB_INTEROP_TOXIPROXY_PYMODBUS",
        query: "",
        // fixtures/modbus/interop/register-map.toml: holding register 13 is 42424.
        address: "holding-register:13:UINT",
        expected: 42424,
        // Holding register 2 is the BYTE 42 of MB-INTEROP-HR-02.
        other_address: "holding-register:2:UINT",
        other_expected: 42,
        block: 1,
        container: "stonecrab-toxi-pymodbus",
    };

    const LIBMODBUS: Target = Target {
        name: "libmodbus",
        variable: "STONECRAB_INTEROP_TOXIPROXY_LIBMODBUS",
        query: "default-unit-identifier=17",
        // unit-test.h, BSD-3-Clause: UT_INPUT_REGISTERS_ADDRESS 0x108 holds 0x000A; a
        // Stonecrab address is the wire address plus one.
        address: "input-register:265:UINT",
        expected: 10,
        // A holding register of the block the sliced read uses: the server starts it at
        // zero, which differs from the 10 of a stale reply.
        other_address: "holding-register:353:UINT",
        other_expected: 0,
        // unit-test.h: UT_REGISTERS_ADDRESS 0x160 (352), plus one.
        block: 353,
        container: "stonecrab-toxi-libmodbus",
    };

    /// Both peers, in the order the tests visit them.
    const TARGETS: [&Target; 2] = [&PYMODBUS, &LIBMODBUS];

    /// The size of the reply to a read of one register: seven bytes of MBAP header, the
    /// function code, the byte count and the register.
    const REPLY_BYTES: u32 = 11;

    /// One HTTP exchange with the Toxiproxy API: the whole reply, or why there is none. Any
    /// status other than 2xx is an error.
    fn request(method: &str, path: &str, body: &str) -> Result<String, String> {
        let address = std::env::var(API).map_err(|_unset| {
            format!(
                "{API} is not set: these tests need Toxiproxy and the peers of \
                 interop/toxiproxy (run `mise run interop-toxiproxy`)"
            )
        })?;
        let mut stream = TcpStream::connect(&address)
            .map_err(|error| format!("cannot reach the Toxiproxy API at {address}: {error}"))?;
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .map_err(|error| error.to_string())?;
        let request = format!(
            "{method} {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\
             Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        stream
            .write_all(request.as_bytes())
            .map_err(|error| error.to_string())?;
        let mut reply = String::new();
        stream
            .read_to_string(&mut reply)
            .map_err(|error| error.to_string())?;
        if reply.starts_with("HTTP/1.1 2") {
            Ok(reply)
        } else {
            Err(format!("{method} {path} {body}: {reply}"))
        }
    }

    /// As [`request`], and a failure fails the test.
    fn api(method: &str, path: &str, body: &str) {
        if let Err(message) = request(method, path, body) {
            panic!("{message}");
        }
    }

    /// Run `docker` with `args`; a failure fails the test with the daemon's message.
    fn docker(args: &[&str]) {
        let output = Command::new("docker")
            .args(args)
            .output()
            .unwrap_or_else(|error| panic!("cannot run docker {args:?}: {error}"));
        assert!(
            output.status.success(),
            "docker {args:?}: {}",
            std::str::from_utf8(&output.stderr).unwrap_or("(not UTF-8)")
        );
    }

    /// Leaves the proxies clean: no toxic, every proxy enabled. Taken at the start of a test
    /// and again at its end, also when the test fails.
    struct Clean;

    impl Clean {
        fn new() -> Self {
            api("POST", "/reset", "");
            Self
        }
    }

    impl Drop for Clean {
        fn drop(&mut self) {
            // Not `api`: a panic in a drop that runs during a failed test would abort.
            let _reset = request("POST", "/reset", "");
        }
    }

    /// Puts a paused or killed container back, whatever the test did, so that the next test
    /// finds the peer running. Both commands fail harmlessly on a container that is running.
    struct Restore(&'static str);

    impl Drop for Restore {
        fn drop(&mut self) {
            let _unpaused = Command::new("docker").args(["unpause", self.0]).output();
            let _started = Command::new("docker").args(["start", self.0]).output();
        }
    }

    /// The driver connected to a peer through its proxy.
    struct Rig {
        target: &'static Target,
        peer: Peer,
        _clean: Clean,
    }

    impl Rig {
        /// Connect and read once, so that the peer is known to answer. A libmodbus that is
        /// still starting again after the previous test is waited for here, as `recover`
        /// does after a fault.
        async fn open(target: &'static Target) -> Self {
            let clean = Clean::new();
            let base = std::env::var(target.variable).unwrap_or_else(|_| {
                panic!(
                    "{} is not set (run `mise run interop-toxiproxy`)",
                    target.variable
                )
            });
            let url = if target.query.is_empty() {
                base
            } else {
                format!("{base}?{}", target.query)
            };
            let hint = format!(
                "Start the containers with `mise run interop-toxiproxy` ({} behind {})",
                target.name, target.variable
            );
            let peer = Peer::connect(&url, &hint).await;
            let rig = Self {
                target,
                peer,
                _clean: clean,
            };
            rig.recover(Duration::from_secs(5), 25).await;
            rig
        }

        fn name(&self) -> &'static str {
            self.target.name
        }

        fn tag(&self) -> Tag {
            self.peer.tag("probe", self.target.address)
        }

        /// One read of the documented register that must finish within `within`.
        async fn read(&self, within: Duration) -> Result<ReadResponse, PlcError> {
            self.peer.try_read(vec![self.tag()], within).await
        }

        /// One read of the other documented register, which must succeed within `within` and
        /// return its own value: a stale reply to the earlier request (the first register's
        /// value) would be a different number.
        async fn read_other(&self, within: Duration) {
            let tag = self.peer.tag("other", self.target.other_address);
            let response = self
                .peer
                .try_read(vec![tag], within)
                .await
                .unwrap_or_else(|error| panic!("{}: the other register: {error}", self.name()));
            assert_eq!(
                value_of(&response, "other"),
                Some(PlcValue::U16(self.target.other_expected)),
                "{}: the other register holds its own value, not a stale reply",
                self.name()
            );
        }

        /// A read that is expected to fail: its error kind and the time it took.
        async fn failing_read(&self, within: Duration) -> (ErrorKind, Duration) {
            let started = Instant::now();
            let outcome = self.read(within).await;
            let elapsed = started.elapsed();
            match outcome {
                Ok(response) => panic!(
                    "{}: the read was expected to fail, it returned {:?}",
                    self.name(),
                    value_of(&response, "probe")
                ),
                Err(error) => (error.kind(), elapsed),
            }
        }

        /// Read until the documented register comes back, allowing `Disconnected` and no
        /// other failure, at most `attempts` times with a real 200 ms pause after each
        /// failure. Returns the failures before the success; each read has `within`, which
        /// must exceed the longest backoff the failures can have built up.
        async fn recover(&self, within: Duration, attempts: usize) -> usize {
            for failures in 0..attempts {
                match self.read(within).await {
                    Ok(response) => {
                        assert_eq!(
                            value_of(&response, "probe"),
                            Some(PlcValue::U16(self.target.expected)),
                            "{}: the documented register",
                            self.name()
                        );
                        return failures;
                    }
                    Err(error) => {
                        assert_eq!(
                            error.kind(),
                            ErrorKind::Disconnected,
                            "{}: recovery allows Disconnected only: {error}",
                            self.name()
                        );
                        tokio::time::sleep(Duration::from_millis(200)).await;
                    }
                }
            }
            panic!("{}: no read succeeded in {attempts} attempts", self.name());
        }

        /// The proxy of this peer.
        fn proxy(&self) -> String {
            format!("/proxies/{}", self.target.name)
        }

        fn add_toxic(&self, name: &str, kind: &str, stream: &str, attributes: &str) {
            let body = format!(
                r#"{{"name":"{name}","type":"{kind}","stream":"{stream}","toxicity":1.0,"attributes":{attributes}}}"#
            );
            api("POST", &format!("{}/toxics", self.proxy()), &body);
        }

        fn remove_toxic(&self, name: &str) {
            api("DELETE", &format!("{}/toxics/{name}", self.proxy()), "");
        }

        async fn close(self) {
            self.peer.close().await;
        }
    }

    /// A timeout is typed `Timeout` and ends at the deadline: not before it, and within a
    /// second after it.
    fn assert_at_deadline(who: &str, kind: ErrorKind, elapsed: Duration, within: Duration) {
        assert_eq!(kind, ErrorKind::Timeout, "{who}: the typed outcome");
        assert!(
            elapsed >= within.saturating_sub(Duration::from_millis(20)),
            "{who}: failed after {elapsed:?}, before the deadline {within:?}"
        );
        assert!(
            elapsed < within.saturating_add(Duration::from_secs(1)),
            "{who}: failed after {elapsed:?}, long after the deadline {within:?}"
        );
    }

    /// A lost link is typed `Disconnected` and is noticed inside the deadline.
    fn assert_disconnected(who: &str, kind: ErrorKind, elapsed: Duration, within: Duration) {
        assert_eq!(kind, ErrorKind::Disconnected, "{who}: the typed outcome");
        assert!(
            elapsed < within,
            "{who}: noticed after {elapsed:?}, not within the deadline {within:?}"
        );
    }

    const SHORT: Duration = Duration::from_millis(500);
    const LONG: Duration = Duration::from_secs(2);
    /// The deadline of a read that waits out a backoff of a few seconds.
    const PATIENT: Duration = Duration::from_secs(10);

    // covers: SPEC mandatory-failure-cases / device restart, network outage, recovery;
    // MB-READ-RECONNECT, TRANSPORT-RECONNECT, MB-TCP-CONNECT (a reset_peer toxic in each
    // direction: the request is lost either way)
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "interop: set STONECRAB_INTEROP_TOXIPROXY"]
    async fn interop_toxi_a_reset_on_the_path_is_disconnected_and_the_next_request_reconnects() {
        for target in TARGETS {
            for stream in ["downstream", "upstream"] {
                let rig = Rig::open(target).await;
                rig.add_toxic("reset", "reset_peer", stream, r#"{"timeout":0}"#);
                let (kind, elapsed) = rig.failing_read(LONG).await;
                assert_disconnected(&format!("{} {stream}", target.name), kind, elapsed, LONG);
                rig.remove_toxic("reset");
                let failures = rig.recover(PATIENT, 5).await;
                assert!(
                    failures <= 2,
                    "{}: reconnected after {failures}",
                    target.name
                );
                rig.close().await;
            }
        }
    }

    // covers: SPEC mandatory-failure-cases / delayed replies after timeout, network outage;
    // MB-READ-TIMEOUT, TRANSPORT-DEADLINES (a timeout toxic with no timeout is a black hole)
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "interop: set STONECRAB_INTEROP_TOXIPROXY"]
    async fn interop_toxi_a_black_hole_times_out_at_the_deadline_and_the_link_recovers() {
        for target in TARGETS {
            for stream in ["downstream", "upstream"] {
                let rig = Rig::open(target).await;
                rig.add_toxic("hole", "timeout", stream, r#"{"timeout":0}"#);
                let who = format!("{} {stream}", target.name);
                let (kind, elapsed) = rig.failing_read(SHORT).await;
                assert_at_deadline(&who, kind, elapsed, SHORT);
                // A second request meets the same silence at its own deadline.
                let (kind, elapsed) = rig.failing_read(SHORT).await;
                assert_at_deadline(&who, kind, elapsed, SHORT);
                rig.remove_toxic("hole");
                let failures = rig.recover(PATIENT, 5).await;
                assert!(failures <= 2, "{who}: recovered after {failures} failures");
                rig.close().await;
            }
        }
    }

    // covers: MB-READ-TIMEOUT, TRANSPORT-DEADLINES (latency with jitter below the deadline)
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "interop: set STONECRAB_INTEROP_TOXIPROXY"]
    async fn interop_toxi_latency_below_the_deadline_still_answers() {
        for target in TARGETS {
            let rig = Rig::open(target).await;
            rig.add_toxic(
                "slow",
                "latency",
                "downstream",
                r#"{"latency":300,"jitter":100}"#,
            );
            let started = Instant::now();
            let response = rig.read(LONG).await.expect("the read completes");
            let elapsed = started.elapsed();
            assert_eq!(
                value_of(&response, "probe"),
                Some(PlcValue::U16(target.expected)),
                "{}: the documented register",
                target.name
            );
            assert!(
                elapsed >= Duration::from_millis(200) && elapsed < LONG,
                "{}: the reply took {elapsed:?}, between 200 ms and the deadline",
                target.name
            );
            rig.close().await;
        }
    }

    // covers: SPEC mandatory-failure-cases / delayed replies after timeout; MB-READ-TIMEOUT,
    // TRANSPORT-DEADLINES, TRANSPORT-RESYNC (latency with jitter above the deadline; the late
    // reply is discarded and the connection is reused)
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "interop: set STONECRAB_INTEROP_TOXIPROXY"]
    async fn interop_toxi_latency_above_the_deadline_times_out_and_the_late_reply_is_discarded() {
        for target in TARGETS {
            let rig = Rig::open(target).await;
            rig.add_toxic(
                "slow",
                "latency",
                "downstream",
                r#"{"latency":1500,"jitter":200}"#,
            );
            let (kind, elapsed) = rig.failing_read(SHORT).await;
            assert_at_deadline(target.name, kind, elapsed, SHORT);
            // Let the delayed reply reach the driver's socket before the toxic goes.
            tokio::time::sleep(Duration::from_millis(1500)).await;
            rig.remove_toxic("slow");
            // Removing a latency toxic does not close the link: the next request, for a
            // different register, is answered on the same connection with its own value,
            // so the reply that was already waiting was discarded.
            rig.read_other(LONG).await;
            let failures = rig.recover(LONG, 1).await;
            assert_eq!(failures, 0, "{}: the connection is reused", target.name);
            rig.close().await;
        }
    }

    // covers: SPEC mandatory-failure-cases / TCP fragmentation; TRANSPORT-STREAM-DELIVERY,
    // MB-READ-FRAMING (a slicer toxic in both directions: requests and replies arrive in
    // pieces of a few bytes)
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "interop: set STONECRAB_INTEROP_TOXIPROXY"]
    async fn interop_toxi_sliced_frames_are_reassembled() {
        for target in TARGETS {
            let rig = Rig::open(target).await;
            let tags = |peer: &Peer| -> Vec<Tag> {
                (0..10_u16)
                    .map(|offset| {
                        let logical = target.block.saturating_add(offset);
                        peer.tag(
                            &format!("block-{offset}"),
                            &format!("holding-register:{logical}:UINT"),
                        )
                    })
                    .collect()
            };
            let whole = rig
                .peer
                .try_read(tags(&rig.peer), LONG)
                .await
                .expect("the unsliced read completes");
            for stream in ["downstream", "upstream"] {
                rig.add_toxic(
                    stream,
                    "slicer",
                    stream,
                    r#"{"average_size":3,"size_variation":2,"delay":2000}"#,
                );
            }
            let started = Instant::now();
            let sliced = rig
                .peer
                .try_read(tags(&rig.peer), LONG)
                .await
                .expect("the sliced read completes");
            let elapsed = started.elapsed();
            assert!(
                elapsed >= Duration::from_millis(10),
                "{}: the slicer's delay was applied ({elapsed:?})",
                target.name
            );
            assert!(elapsed < LONG, "{}: within the deadline", target.name);
            for offset in 0..10 {
                let id = format!("block-{offset}");
                assert_eq!(
                    value_of(&sliced, &id),
                    value_of(&whole, &id),
                    "{}: {id} reads the same sliced and whole",
                    target.name
                );
                assert!(
                    value_of(&sliced, &id).is_some(),
                    "{}: {id} was read",
                    target.name
                );
            }
            rig.close().await;
        }
    }

    // covers: SPEC mandatory-failure-cases / peer close at each framing boundary;
    // MB-TCP-CONNECT, TRANSPORT-RECONNECT (limit_data: the link closes after one whole reply)
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "interop: set STONECRAB_INTEROP_TOXIPROXY"]
    async fn interop_toxi_limit_data_closes_the_connection_after_the_bytes() {
        for target in TARGETS {
            let rig = Rig::open(target).await;
            rig.add_toxic(
                "limit",
                "limit_data",
                "downstream",
                &format!(r#"{{"bytes":{REPLY_BYTES}}}"#),
            );
            let response = rig.read(LONG).await.expect("the one reply that is allowed");
            assert_eq!(
                value_of(&response, "probe"),
                Some(PlcValue::U16(target.expected)),
                "{}: the reply that fits the limit",
                target.name
            );
            let (kind, elapsed) = rig.failing_read(LONG).await;
            assert_disconnected(target.name, kind, elapsed, LONG);
            rig.remove_toxic("limit");
            let failures = rig.recover(PATIENT, 5).await;
            assert!(failures <= 2, "{}: recovered after {failures}", target.name);
            rig.close().await;
        }
    }

    // covers: SPEC mandatory-failure-cases / truncated headers, peer close at each framing
    // boundary; MB-READ-FRAMING, TRANSPORT-FRAMING (limit_data inside the header: four bytes
    // of the request or of the reply, then the link closes)
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "interop: set STONECRAB_INTEROP_TOXIPROXY"]
    async fn interop_toxi_a_frame_cut_inside_its_header_is_disconnected() {
        for target in TARGETS {
            for stream in ["downstream", "upstream"] {
                let rig = Rig::open(target).await;
                rig.add_toxic("cut", "limit_data", stream, r#"{"bytes":4}"#);
                let (kind, elapsed) = rig.failing_read(LONG).await;
                assert_disconnected(&format!("{} {stream}", target.name), kind, elapsed, LONG);
                rig.remove_toxic("cut");
                let failures = rig.recover(PATIENT, 5).await;
                assert!(failures <= 2, "{}: recovered after {failures}", target.name);
                rig.close().await;
            }
        }
    }

    // covers: SPEC mandatory-failure-cases / network outage, recovery; MB-READ-RECONNECT,
    // TRANSPORT-RECONNECT (disabling the proxy closes every open connection)
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "interop: set STONECRAB_INTEROP_TOXIPROXY"]
    async fn interop_toxi_disabling_the_proxy_closes_every_connection_and_enabling_it_reconnects() {
        // pymodbus serves several connections at once; libmodbus one, so it has no second.
        let first = Rig::open(&PYMODBUS).await;
        let second = Rig::open(&PYMODBUS).await;
        api("POST", &first.proxy(), r#"{"enabled":false}"#);
        for rig in [&first, &second] {
            let (kind, elapsed) = rig.failing_read(LONG).await;
            assert_disconnected("pymodbus", kind, elapsed, LONG);
        }
        // While the proxy is off a reconnect cannot work either: still Disconnected.
        let (kind, elapsed) = first.failing_read(PATIENT).await;
        assert_disconnected("pymodbus, proxy still off", kind, elapsed, PATIENT);
        api("POST", &first.proxy(), r#"{"enabled":true}"#);
        for rig in [&first, &second] {
            let failures = rig.recover(PATIENT, 8).await;
            assert!(failures <= 3, "reconnected after {failures} failures");
        }
        first.close().await;
        second.close().await;

        let rig = Rig::open(&LIBMODBUS).await;
        api("POST", &rig.proxy(), r#"{"enabled":false}"#);
        let (kind, elapsed) = rig.failing_read(LONG).await;
        assert_disconnected("libmodbus", kind, elapsed, LONG);
        api("POST", &rig.proxy(), r#"{"enabled":true}"#);
        let failures = rig.recover(PATIENT, 8).await;
        assert!(
            failures <= 3,
            "libmodbus reconnected after {failures} failures"
        );
        rig.close().await;
    }

    // covers: SPEC mandatory-failure-cases / delayed replies after timeout, device restart;
    // MB-READ-TIMEOUT, TRANSPORT-DEADLINES, TRANSPORT-RESYNC (docker pause: the peer is
    // alive at TCP level and silent at the application level, then answers late)
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "interop: set STONECRAB_INTEROP_TOXIPROXY"]
    async fn interop_toxi_a_paused_peer_times_out_and_answers_late() {
        let rig = Rig::open(&PYMODBUS).await;
        let _restore = Restore(PYMODBUS.container);
        docker(&["pause", PYMODBUS.container]);
        let (kind, elapsed) = rig.failing_read(SHORT).await;
        assert_at_deadline("paused pymodbus", kind, elapsed, SHORT);
        docker(&["unpause", PYMODBUS.container]);
        // The connection never closed: the peer's late reply to the abandoned request is
        // discarded and the next request is answered on the same connection.
        rig.read_other(LONG).await;
        let failures = rig.recover(LONG, 1).await;
        assert_eq!(failures, 0, "the timed-out connection is reused");
        rig.close().await;
    }

    // covers: SPEC mandatory-failure-cases / device restart, network outage, recovery;
    // MB-READ-RECONNECT, TRANSPORT-RECONNECT, MB-TCP-CONNECT (docker kill, then docker start)
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "interop: set STONECRAB_INTEROP_TOXIPROXY"]
    async fn interop_toxi_a_killed_peer_is_disconnected_and_the_driver_returns_when_it_restarts() {
        let rig = Rig::open(&PYMODBUS).await;
        let _restore = Restore(PYMODBUS.container);
        docker(&["kill", PYMODBUS.container]);
        let (kind, elapsed) = rig.failing_read(LONG).await;
        assert_disconnected("killed pymodbus", kind, elapsed, LONG);
        // The proxy still accepts, finds nobody upstream and closes: Disconnected again.
        let (kind, elapsed) = rig.failing_read(PATIENT).await;
        assert_disconnected("killed pymodbus, second request", kind, elapsed, PATIENT);
        docker(&["start", PYMODBUS.container]);
        // The server needs a second or two to listen; the budget is generous because the
        // backoff grows while it does.
        let started = Instant::now();
        let failures = rig.recover(PATIENT, 20).await;
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "back after {failures} failures and {:?}",
            started.elapsed()
        );
        rig.close().await;
    }
}
