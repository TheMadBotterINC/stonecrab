//! Interoperability over UDP (slice S61, ADR-0027): the driver's `modbus-tcp:udp://`
//! transport against diagslave 3.6, the free Modbus slave simulator of proconX
//! (`diagslave -m udp`), in a container built from `interop/diagslave/`, with `tc netem`
//! inside the container losing, delaying, duplicating and reordering its replies.
//!
//! Every test here is `#[ignore]`d and runs only when `STONECRAB_INTEROP_DIAGSLAVE` names the
//! server (for example `modbus-tcp:udp://127.0.0.1:1502`): `mise run interop-udp` (and `mise
//! run interop`) starts the container and runs them, and so does the `interop-udp` job of
//! `.github/workflows/ci.yml`. Without the variable the tests stay ignored and nothing here
//! touches the network.
//!
//! # How the faults are made
//! The test process is the controller, as in the Toxiproxy run: it runs `docker exec` as root
//! in the container (which has the `NET_ADMIN` capability) to put a `netem` queueing
//! discipline on the container's `eth0` and to remove it again. `netem` acts on what leaves
//! that interface, so it shapes the replies; the requests reach the simulator untouched.
//! Each test begins and ends with no discipline, also when it fails (a discipline that
//! cannot be removed fails the test), and the tests run one at a time (`--test-threads 1`),
//! because the discipline belongs to the container.
//!
//! # What the server is
//! diagslave answers every unit id, keeps its registers and coils in memory, zero until
//! written, and logs each request. Each test writes the values it reads first, with no
//! discipline in place. The values are chosen here; nothing is read from the image.
//!
//! # Time
//! The deadlines are real (the system clock the driver uses) and `netem` delays by real
//! milliseconds. The bounds asserted are: a timeout is not earlier than its deadline and not
//! later than the deadline plus a second; a reply delayed by 300 ms takes at least 250 ms.
//! No test sleeps.
#![cfg(all(feature = "modbus", feature = "testkit"))]
#![forbid(unsafe_code)]

#[cfg(test)]
#[path = "support/interop.rs"]
#[expect(
    dead_code,
    reason = "the module is shared with the other interop files; these tests read no failed item"
)]
mod interop;

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
    use std::process::Command;
    use std::time::{Duration, Instant};

    use stonecrab::{ErrorKind, PlcValue};

    use crate::interop::{Peer, value_of};

    /// The environment variable that names the server.
    const VARIABLE: &str = "STONECRAB_INTEROP_DIAGSLAVE";

    /// The container `interop/diagslave/interop.sh` starts.
    const CONTAINER: &str = "stonecrab-interop-diagslave";

    /// Three holding registers of the write area these tests use, with the values each test
    /// writes into them first: distinct, so a reply to the wrong request shows.
    const REGISTERS: [(&str, u16); 3] = [
        ("holding-register:101:UINT", 0x1111),
        ("holding-register:102:UINT", 0x2222),
        ("holding-register:103:UINT", 0x3333),
    ];

    fn url() -> String {
        let base = std::env::var(VARIABLE).unwrap_or_else(|_| {
            panic!(
                "{VARIABLE} is not set: these tests need the diagslave container of \
                 interop/diagslave (run `mise run interop-udp`, or start the container and set \
                 {VARIABLE}=modbus-tcp:udp://127.0.0.1:1502)"
            )
        });
        format!("{base}?access=read-write&request-timeout-ms=2000")
    }

    /// Removes the root discipline of `eth0`.
    const DELETE: [&str; 10] = [
        "exec", "-u", "0", CONTAINER, "tc", "qdisc", "del", "dev", "eth0", "root",
    ];

    /// Run `docker` with `args` and return what it printed; a failure fails the test with the
    /// daemon's message.
    fn docker(args: &[&str]) -> String {
        let output = Command::new("docker")
            .args(args)
            .output()
            .unwrap_or_else(|error| panic!("cannot run docker {args:?}: {error}"));
        assert!(
            output.status.success(),
            "docker {args:?}: {}",
            std::str::from_utf8(&output.stderr).unwrap_or("(not UTF-8)")
        );
        std::str::from_utf8(&output.stdout)
            .unwrap_or_else(|error| {
                panic!("docker {args:?} printed text that is not UTF-8: {error}")
            })
            .to_owned()
    }

    /// The queueing disciplines of the container's `eth0`, as `tc` lists them.
    fn disciplines() -> String {
        docker(&[
            "exec", "-u", "0", CONTAINER, "tc", "qdisc", "show", "dev", "eth0",
        ])
    }

    /// Removes the `netem` discipline of `eth0` if there is one, and checks that none is
    /// left; every docker or tc failure fails the test.
    fn clear() {
        if disciplines().contains("netem") {
            docker(&DELETE);
        }
        let left = disciplines();
        assert!(
            !left.contains("netem"),
            "a netem discipline is still on eth0: {left}"
        );
    }

    /// Put `netem` with these parameters on the replies of the container.
    fn netem(parameters: &str) {
        let mut args = vec![
            "exec", "-u", "0", CONTAINER, "tc", "qdisc", "replace", "dev", "eth0", "root", "netem",
        ];
        args.extend(parameters.split_whitespace());
        docker(&args);
    }

    /// No discipline from the start of a test to its end, whatever the test applied: one
    /// that cannot be removed fails the test.
    struct Clean;

    impl Clean {
        fn new() -> Self {
            clear();
            Self
        }
    }

    impl Drop for Clean {
        fn drop(&mut self) {
            if std::thread::panicking() {
                // Not `clear`: a panic in a drop that runs during a failed test would abort.
                // The next test's `Clean::new` removes what this leaves behind, and checks.
                let _removed = Command::new("docker").args(DELETE).output();
            } else {
                clear();
            }
        }
    }

    /// Connect, and write the three registers with no discipline in place.
    async fn prepared() -> Peer {
        let peer = Peer::connect(
            &url(),
            "Start it with `mise run interop-udp` or point the variable at a diagslave -m udp",
        )
        .await;
        let items = REGISTERS
            .iter()
            .map(|(address, value)| (peer.tag(address, address), PlcValue::U16(*value)))
            .collect();
        let written = peer.write(items).await;
        assert_eq!(written.confirmed_count(), REGISTERS.len(), "{written:?}");
        peer
    }

    /// Read one register within `within`: its value, or the error of the whole read.
    async fn read(peer: &Peer, address: &str, within: Duration) -> Result<PlcValue, ErrorKind> {
        match peer.try_read(vec![peer.tag("x", address)], within).await {
            Ok(response) => Ok(value_of(&response, "x").expect("the item was read")),
            Err(error) => Err(error.kind()),
        }
    }

    // covers: S61 / MB-TCP-TRANSPORTS-UDP reads, writes and ping against an independent server
    #[tokio::test]
    #[ignore = "interop: set STONECRAB_INTEROP_DIAGSLAVE"]
    async fn interop_udp_writes_and_reads_back_registers_and_coils() {
        let _clean = Clean::new();
        let peer = prepared().await;
        let tags = REGISTERS
            .iter()
            .map(|(address, _)| peer.tag(address, address))
            .collect();
        let response = peer.read(tags).await;
        for (address, value) in REGISTERS {
            assert_eq!(
                value_of(&response, address),
                Some(PlcValue::U16(value)),
                "{address}"
            );
        }
        let coil = peer.tag("coil", "coil:7");
        let written = peer.write(vec![(coil.clone(), PlcValue::Bool(true))]).await;
        assert_eq!(written.confirmed_count(), 1, "{written:?}");
        let response = peer.read(vec![coil]).await;
        assert_eq!(value_of(&response, "coil"), Some(PlcValue::Bool(true)));
        peer.connection
            .ping()
            .await
            .expect("ping reads the ping address");
        peer.close().await;
    }

    // covers: S61 / MB-TCP-TRANSPORTS-UDP netem loss: every reply lost, Timeout at the deadline
    #[tokio::test]
    #[ignore = "interop: set STONECRAB_INTEROP_DIAGSLAVE"]
    async fn interop_udp_netem_loss_of_every_reply_times_out_at_the_deadline() {
        let _clean = Clean::new();
        let peer = prepared().await;
        let (address, value) = REGISTERS[0];
        netem("loss 100%");
        let deadline = Duration::from_millis(500);
        for _ in 0..2 {
            let started = Instant::now();
            let outcome = read(&peer, address, deadline).await;
            let elapsed = started.elapsed();
            assert_eq!(outcome, Err(ErrorKind::Timeout), "a lost reply");
            assert!(elapsed >= deadline, "not before the deadline: {elapsed:?}");
            assert!(elapsed < deadline + Duration::from_secs(1), "{elapsed:?}");
        }
        clear();
        assert_eq!(
            read(&peer, address, Duration::from_secs(2)).await,
            Ok(PlcValue::U16(value)),
            "the next request is answered once the replies get through"
        );
        peer.close().await;
    }

    // covers: S61 / MB-TCP-TRANSPORTS-UDP netem delay below the deadline
    #[tokio::test]
    #[ignore = "interop: set STONECRAB_INTEROP_DIAGSLAVE"]
    async fn interop_udp_netem_delay_below_the_deadline_still_answers() {
        let _clean = Clean::new();
        let peer = prepared().await;
        netem("delay 300ms");
        for (address, value) in REGISTERS {
            let started = Instant::now();
            let outcome = read(&peer, address, Duration::from_secs(2)).await;
            let elapsed = started.elapsed();
            assert_eq!(outcome, Ok(PlcValue::U16(value)), "{address}");
            assert!(
                elapsed >= Duration::from_millis(250),
                "delayed: {elapsed:?}"
            );
        }
        peer.close().await;
    }

    // covers: SPEC mandatory-failure-cases / F2 late responses; S61 / netem delay past the deadline
    #[tokio::test]
    #[ignore = "interop: set STONECRAB_INTEROP_DIAGSLAVE"]
    async fn interop_udp_netem_delay_past_the_deadline_times_out_and_the_late_reply_is_discarded() {
        let _clean = Clean::new();
        let peer = prepared().await;
        let (first, _) = REGISTERS[0];
        let (second, value) = REGISTERS[1];
        netem("delay 1000ms");
        assert_eq!(
            read(&peer, first, Duration::from_millis(400)).await,
            Err(ErrorKind::Timeout),
            "the reply comes after the deadline"
        );
        // The first reply arrives while this request waits, a second before its own.
        assert_eq!(
            read(&peer, second, Duration::from_secs(3)).await,
            Ok(PlcValue::U16(value)),
            "the late reply of the first request is not taken for the second"
        );
        peer.close().await;
    }

    // covers: SPEC mandatory-failure-cases / F2 late responses; S61 / netem duplication
    #[tokio::test]
    #[ignore = "interop: set STONECRAB_INTEROP_DIAGSLAVE"]
    async fn interop_udp_netem_duplicated_replies_are_discarded_by_correlation() {
        let _clean = Clean::new();
        let peer = prepared().await;
        netem("duplicate 100%");
        // The premise, checked: one raw request (FC03, one register, transaction id 0xBEEF)
        // gets two identical replies, so the reads below do meet copies.
        let base = std::env::var(VARIABLE).expect("prepared read it");
        let server = base
            .strip_prefix("modbus-tcp:udp://")
            .and_then(|rest| rest.split(['/', '?']).next())
            .expect("a modbus-tcp:udp://host:port URL");
        let target = tokio::net::lookup_host(server)
            .await
            .expect("resolve")
            .next()
            .expect("an address");
        let local: SocketAddr = if target.is_ipv4() {
            (Ipv4Addr::UNSPECIFIED, 0).into()
        } else {
            (Ipv6Addr::UNSPECIFIED, 0).into()
        };
        let probe = tokio::net::UdpSocket::bind(local).await.expect("bind");
        probe.connect(target).await.expect("connect");
        probe
            .send(&[0xBE, 0xEF, 0, 0, 0, 6, 1, 0x03, 0, 100, 0, 1])
            .await
            .expect("send");
        let (mut first, mut second) = ([0_u8; 32], [0_u8; 32]);
        let within = Duration::from_secs(2);
        let one = tokio::time::timeout(within, probe.recv(&mut first))
            .await
            .expect("a reply to the probe")
            .expect("recv");
        let two = tokio::time::timeout(within, probe.recv(&mut second))
            .await
            .expect("netem sends every reply twice")
            .expect("recv");
        assert_eq!(first.get(..2), Some(&[0xBE, 0xEF][..]), "the probe's reply");
        assert_eq!(first.get(..one), second.get(..two), "and its copy");
        for _ in 0..4 {
            for (address, value) in REGISTERS {
                assert_eq!(
                    read(&peer, address, Duration::from_secs(2)).await,
                    Ok(PlcValue::U16(value)),
                    "{address}: never the copy of an earlier reply"
                );
            }
        }
        peer.close().await;
    }

    // covers: SPEC mandatory-failure-cases / F2 late responses; S61 / netem reordering
    #[tokio::test]
    #[ignore = "interop: set STONECRAB_INTEROP_DIAGSLAVE"]
    async fn interop_udp_netem_reordered_replies_never_answer_the_wrong_request() {
        let _clean = Clean::new();
        let peer = prepared().await;
        // Half the replies leave at once, the other half 400 ms later, after the deadline of
        // their request and while a later request waits: they overtake each other.
        netem("delay 400ms reorder 50%");
        let (mut answered, mut timed_out) = (0_u32, 0_u32);
        for round in 0..30_usize {
            let Some((address, value)) = REGISTERS.get(round % REGISTERS.len()) else {
                continue;
            };
            match read(&peer, address, Duration::from_millis(250)).await {
                Ok(read) => {
                    assert_eq!(read, PlcValue::U16(*value), "{address} in round {round}");
                    answered += 1;
                }
                Err(kind) => {
                    assert_eq!(kind, ErrorKind::Timeout, "round {round}");
                    timed_out += 1;
                }
            }
        }
        assert!(
            answered > 0 && timed_out > 0,
            "{answered} answered, {timed_out} timed out"
        );
        peer.close().await;
    }
}
