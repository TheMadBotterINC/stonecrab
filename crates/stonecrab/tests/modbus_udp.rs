//! Modbus TCP over UDP (`modbus-tcp:udp://`, slice S61, ADR-0027) against a first-party
//! device on a loopback UDP socket: reads and writes, a lost reply, a reply that arrives
//! twice, a reply that arrives after its request timed out, a reply split across two
//! datagrams, and the common driver contract suite.
//!
//! The device is the MBAP responder of the contract runs (`tests/support/responder.rs`) and
//! the interop register map, behind a socket that answers each request datagram with one
//! reply datagram. A test can make it send each reply twice, hold one reply back until the
//! next request arrives, or send one reply in two datagrams; nothing else stands between the
//! driver and the device, and the driver uses its real UDP transport and the system clock.
//! Every socket is on `127.0.0.1`.
//!
//! Nothing waits on real time beyond a bounded socket wait. A request whose reply must not
//! come in time runs on Tokio's paused clock, as the pending calls of `transport_serial.rs`
//! do: while every task is idle the clock moves to the request's deadline at once, and the
//! clock is resumed as soon as the request has failed. UDP has no close, so the responder's
//! `Reaction::Close` (a link that ends mid-reply) is sent as a datagram that ends inside an
//! MBAP header, which the driver fails at once; that is the lost write confirmation of the
//! contract run, as a closed connection is for the loopback TCP run
//! (`modbus_contract_scripted_peer.rs`).
#![cfg(all(feature = "modbus", feature = "testkit"))]
#![forbid(unsafe_code)]

#[cfg(test)]
#[path = "support/responder.rs"]
#[expect(
    dead_code,
    reason = "the module is shared with the contract runs; these tests use no extended register and no disconnect"
)]
mod responder;

#[cfg(test)]
#[path = "support/interop.rs"]
#[expect(
    dead_code,
    reason = "the module is shared with the interop files; these tests read no failed item"
)]
mod interop;

#[cfg(test)]
mod tests {
    use std::io::{self, Write};
    use std::net::SocketAddr;
    use std::path::Path;
    use std::sync::{Arc, Mutex, PoisonError};
    use std::time::Duration;

    use stonecrab::modbus::{ExceptionCode, ModbusTcpDriver, SystemClock};
    use stonecrab::testkit::{
        CaseOutcome, ContractFixture, ContractReport, ContractSuite, FixtureAddress, InteropMap,
        Reaction,
    };
    use stonecrab::{Clock, ErrorKind, PlcError, PlcValue, ReadResponse};
    use tokio::net::UdpSocket;
    use tokio::task::JoinSet;

    use crate::interop::{Peer, value_of};
    use crate::responder::Device;

    /// A holding register of the interop map: logical 13 is the UINT 42424.
    const READABLE: &str = "holding-register:13:UINT";
    const READABLE_VALUE: PlcValue = PlcValue::U16(42_424);

    /// Another one with another value: logical 12 is the INT -2424.
    const OTHER: &str = "holding-register:12:INT";
    const OTHER_VALUE: PlcValue = PlcValue::I16(-2_424);

    /// An address far outside the 201-register block: exception 02.
    const REJECTED: &str = "holding-register:60000:UINT";

    /// A holding register of the write area 100 to 200, which starts at zero.
    const WRITABLE: &str = "holding-register:100:UINT";

    /// The deadline of a request whose reply must not come in time (on the paused clock).
    const SHORT: Duration = Duration::from_millis(300);

    /// The connection's request timeout, longer than `SHORT`.
    const REQUEST_TIMEOUT: &str = "request-timeout-ms=2000";

    /// The length of an MBAP header: where a split reply is cut.
    const MBAP_HEADER: usize = 7;

    fn register_map() -> InteropMap {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/modbus/interop/register-map.toml");
        InteropMap::load(&path).expect("the interop register map loads")
    }

    /// What the socket does with a reply besides sending it once.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Twist {
        /// Send it once.
        Plain,
        /// Send it twice, back to back.
        Twice,
        /// Keep it until the next request arrives, then send it before that request's reply.
        HoldNext,
        /// Send its MBAP header and the rest as two datagrams, once.
        Split,
    }

    /// The device on a loopback UDP socket; its task is aborted when this is dropped.
    struct UdpDevice {
        address: SocketAddr,
        twist: Arc<Mutex<Twist>>,
        _task: JoinSet<()>,
    }

    impl UdpDevice {
        async fn spawn(device: &Device) -> Self {
            let socket = UdpSocket::bind("127.0.0.1:0").await.expect("bind");
            let address = socket.local_addr().expect("address");
            let twist = Arc::new(Mutex::new(Twist::Plain));
            let shared = Arc::clone(&twist);
            let mut react = device.reactor();
            let mut task = JoinSet::new();
            task.spawn(async move {
                let mut held: Option<Vec<u8>> = None;
                let mut datagram = vec![0_u8; 512];
                loop {
                    let (length, client) = match socket.recv_from(&mut datagram).await {
                        Ok(received) => received,
                        // Windows reports the ICMP port-unreachable of a reply sent to a
                        // closed client here, once; the socket goes on.
                        Err(error) if error.kind() == io::ErrorKind::ConnectionReset => continue,
                        Err(_) => return,
                    };
                    let Some(request) = datagram.get(..length) else {
                        continue;
                    };
                    let reply = match react(request) {
                        Reaction::Reply(reply) => reply,
                        // UDP has no close: a link that ends mid-reply leaves a datagram that
                        // ends inside the MBAP header, its transaction and protocol ids.
                        Reaction::Close => request.iter().copied().take(4).collect(),
                        Reaction::Silence | Reaction::ReplyThenClose(_) => continue,
                    };
                    let mode = *shared.lock().unwrap_or_else(PoisonError::into_inner);
                    let mut out = held.take().into_iter().collect::<Vec<_>>();
                    match mode {
                        Twist::Plain => out.push(reply),
                        Twist::Twice => out.extend([reply.clone(), reply]),
                        Twist::HoldNext => held = Some(reply),
                        Twist::Split => match reply.split_at_checked(MBAP_HEADER) {
                            Some((header, rest)) => out.extend([header.to_vec(), rest.to_vec()]),
                            None => out.push(reply),
                        },
                    }
                    if matches!(mode, Twist::HoldNext | Twist::Split) {
                        *shared.lock().unwrap_or_else(PoisonError::into_inner) = Twist::Plain;
                    }
                    for bytes in out {
                        // A client that has gone away misses its reply, as over a network.
                        let _sent = socket.send_to(&bytes, client).await;
                    }
                }
            });
            Self {
                address,
                twist,
                _task: task,
            }
        }

        fn twist(&self, twist: Twist) {
            *self.twist.lock().unwrap_or_else(PoisonError::into_inner) = twist;
        }

        fn uri(&self, query: &str) -> String {
            format!("modbus-tcp:udp://{}?{query}", self.address)
        }
    }

    async fn read_one(peer: &Peer, address: &str) -> Option<PlcValue> {
        let response = peer.read(vec![peer.tag("x", address)]).await;
        value_of(&response, "x")
    }

    /// Read `address` within `SHORT` on Tokio's paused clock, for a reply that must not come
    /// in time: the result and the time that passed on that clock.
    async fn read_paused(peer: &Peer, address: &str) -> (Result<ReadResponse, PlcError>, Duration) {
        tokio::time::pause();
        let started = tokio::time::Instant::now();
        let result = peer.try_read(vec![peer.tag("x", address)], SHORT).await;
        let elapsed = started.elapsed();
        tokio::time::resume();
        (result, elapsed)
    }

    // covers: S61 / MB-TCP-TRANSPORTS-UDP reads and writes over a connected UDP socket
    #[tokio::test]
    async fn reads_and_writes_travel_as_one_datagram_each_way() {
        let device = Device::from_map(&register_map());
        let socket = UdpDevice::spawn(&device).await;
        let peer = Peer::connect(&socket.uri("access=read-write"), "loopback").await;
        assert_eq!(read_one(&peer, READABLE).await, Some(READABLE_VALUE));
        let response = peer
            .read(vec![peer.tag("a", READABLE), peer.tag("b", OTHER)])
            .await;
        assert_eq!(value_of(&response, "a"), Some(READABLE_VALUE));
        assert_eq!(value_of(&response, "b"), Some(OTHER_VALUE));

        let written = peer
            .write(vec![(peer.tag("w", WRITABLE), PlcValue::U16(0x1234))])
            .await;
        assert_eq!(written.confirmed_count(), 1, "the write is confirmed");
        assert_eq!(device.holding(100), Some(0x1234), "the device applied it");
        assert_eq!(read_one(&peer, WRITABLE).await, Some(PlcValue::U16(0x1234)));
        peer.close().await;
    }

    // covers: SPEC mandatory-failure-cases / F2 transport (deadline expiry); S61 / a lost datagram
    #[tokio::test]
    async fn a_lost_reply_is_a_timeout_at_the_deadline() {
        let device = Device::from_map(&register_map());
        let socket = UdpDevice::spawn(&device).await;
        let peer = Peer::connect(&socket.uri(REQUEST_TIMEOUT), "loopback").await;
        device.arm_swallow();
        let (result, elapsed) = read_paused(&peer, READABLE).await;
        let error = result.expect_err("the reply never comes");
        assert_eq!(error.kind(), ErrorKind::Timeout, "{error}");
        assert!(elapsed >= SHORT, "not before the deadline: {elapsed:?}");
        assert!(
            elapsed < Duration::from_secs(2),
            "the request's deadline, not the connection's request timeout: {elapsed:?}"
        );
        assert_eq!(
            read_one(&peer, READABLE).await,
            Some(READABLE_VALUE),
            "next read"
        );
        peer.close().await;
    }

    // covers: SPEC mandatory-failure-cases / F2 late responses; S61 / a duplicated datagram
    #[tokio::test]
    async fn a_duplicated_reply_is_discarded_by_correlation() {
        let device = Device::from_map(&register_map());
        let socket = UdpDevice::spawn(&device).await;
        socket.twist(Twist::Twice);
        let peer = Peer::connect(&socket.uri(REQUEST_TIMEOUT), "loopback").await;
        for _ in 0..3 {
            assert_eq!(read_one(&peer, READABLE).await, Some(READABLE_VALUE));
            assert_eq!(
                read_one(&peer, OTHER).await,
                Some(OTHER_VALUE),
                "never the copy"
            );
        }
        peer.close().await;
    }

    // covers: SPEC mandatory-failure-cases / F2 late responses; S61 / a late datagram
    #[tokio::test]
    async fn a_late_reply_after_a_timeout_does_not_satisfy_the_next_request() {
        let device = Device::from_map(&register_map());
        let socket = UdpDevice::spawn(&device).await;
        let peer = Peer::connect(&socket.uri(REQUEST_TIMEOUT), "loopback").await;
        socket.twist(Twist::HoldNext);
        let (result, _elapsed) = read_paused(&peer, READABLE).await;
        let error = result.expect_err("the reply is held back");
        assert_eq!(error.kind(), ErrorKind::Timeout, "{error}");
        // The held reply (42424) reaches the socket just before this request's own reply.
        assert_eq!(
            read_one(&peer, OTHER).await,
            Some(OTHER_VALUE),
            "the late reply of the first request is not taken for the second"
        );
        assert_eq!(read_one(&peer, READABLE).await, Some(READABLE_VALUE));
        peer.close().await;
    }

    // covers: SPEC mandatory-failure-cases / F2 truncated frames; S61 / UTR-UDP-RING a reply split in two datagrams
    #[tokio::test]
    async fn a_reply_split_across_two_datagrams_is_malformed_and_never_joined() {
        let device = Device::from_map(&register_map());
        let socket = UdpDevice::spawn(&device).await;
        let peer = Peer::connect(&socket.uri(REQUEST_TIMEOUT), "loopback").await;
        socket.twist(Twist::Split);
        let error = peer
            .try_read(vec![peer.tag("x", READABLE)], Duration::from_secs(2))
            .await
            .expect_err("a datagram holding only the MBAP header");
        assert_eq!(error.kind(), ErrorKind::MalformedData, "{error}");
        // The rest of the split reply is the next datagram; it is read alone, never joined to
        // the header before it nor taken as the reply of the next request.
        let error = peer
            .try_read(vec![peer.tag("x", OTHER)], Duration::from_secs(2))
            .await
            .expect_err("the rest of the split reply is no reply");
        assert_eq!(error.kind(), ErrorKind::MalformedData, "{error}");
        assert_eq!(read_one(&peer, OTHER).await, Some(OTHER_VALUE), "next read");
        peer.close().await;
    }

    const ABSENT_DEADLINE: &str = "an absent deadline uses the connection timeout";
    const TIMEOUT_DIAGNOSTICS: &str =
        "a timeout carries diagnostics naming operation, state and category";
    const RETRY_STOPS: &str = "read retry stops after the configured maximum attempts";
    const SILENT_PING: &str = "a ping to a silent device yields Timeout";
    const DISCONNECT: &str = "a disconnect during a read yields Disconnected";
    const DISCONNECT_DIAGNOSTICS: &str =
        "a disconnect carries diagnostics naming operation, state and category";

    /// The cases the fixture cannot run: the timing cases need a deterministic clock (as for
    /// the loopback TCP run), and a UDP device cannot close a connection there is none of.
    const SKIPPED: [(&str, &str); 10] = [
        (ABSENT_DEADLINE, "fixture cannot inject timeout"),
        (TIMEOUT_DIAGNOSTICS, "fixture cannot inject timeout"),
        (RETRY_STOPS, "fixture cannot inject timeout"),
        (SILENT_PING, "fixture cannot inject timeout"),
        (
            "a cyclic subscription delivers one event per tick and discloses its interval",
            "fixture cannot inject timeout",
        ),
        (
            "a change-of-state subscription delivers a first value and then only changes",
            "fixture cannot inject timeout",
        ),
        (
            "unsubscribing stops the polling reads",
            "fixture cannot inject timeout",
        ),
        (
            "a stalled polling read skips the ticks that came due and never bursts",
            "fixture cannot inject timeout",
        ),
        (DISCONNECT, "fixture cannot inject disconnect"),
        (DISCONNECT_DIAGNOSTICS, "fixture cannot inject disconnect"),
    ];

    async fn contract_run() -> (Device, ContractReport) {
        let device = Device::from_map(&register_map());
        let socket = UdpDevice::spawn(&device).await;
        let uri = socket.uri("access=read-write");
        let driver = ModbusTcpDriver::new();
        // Opening a UDP socket sends nothing, so the requests that reached the device are
        // all the I/O there is.
        let probe = device.clone();
        let counter = device.clone();
        let exception = device.clone();
        let unknown = device.clone();
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::default());
        let fixture = ContractFixture::new(
            clock,
            READABLE,
            REJECTED,
            Arc::new(FixtureAddress::new("elsewhere", None)),
            move || u64::from(probe.reads()),
            move || counter.reads(),
        )
        .with_writable(WRITABLE, PlcValue::U16(1))
        .with_inject_unknown_after_transmit(move || unknown.arm_apply_then_disconnect())
        .with_inject_device_exception(move || {
            exception.arm_exception(ExceptionCode::ServerDeviceFailure);
        });
        let report = ContractSuite::run(&driver, &uri, &fixture)
            .await
            .expect("the suite always resolves");
        writeln!(io::stdout().lock(), "{uri}\n{report}").expect("stdout");
        drop(socket);
        (device, report)
    }

    // covers: SPEC test-layers / contract suite against modbus-tcp over UDP loopback sockets
    #[tokio::test(flavor = "current_thread")]
    async fn the_suite_is_green_against_modbus_tcp_over_udp() {
        let (device, report) = contract_run().await;
        assert!(report.is_green(), "the suite is green:\n{report}");
        assert_eq!(report.failed(), 0, "no failures:\n{report}");
        assert_eq!(
            report.passed().saturating_add(report.skipped()),
            ContractSuite::cases().len(),
            "every case has an outcome:\n{report}"
        );
        assert_eq!(
            report.skipped(),
            SKIPPED.len() + 3,
            "the named skips plus unsupported write, ping and subscribe:\n{report}"
        );
        for (name, why) in SKIPPED {
            assert_eq!(
                report.outcome(name),
                Some(&CaseOutcome::Skipped(why.to_owned())),
                "{name}:\n{report}"
            );
        }
        for name in [
            "an unknown write outcome is never replayed",
            "a read-only policy rejects a write with no I/O",
            "a supported ping answers and changes no state",
            "a device exception carries diagnostics naming operation, state and category",
            "read retry skips non-retryable kinds",
            "a read correlates every tag with its result in request order",
            "a partial failure keeps the successes",
            "an elapsed deadline yields Timeout with no I/O",
            "a foreign address is rejected with no I/O",
            "a polling driver refuses an event subscription with no I/O",
            "closing the connection ends its subscriptions after the queued events",
        ] {
            assert_eq!(
                report.outcome(name),
                Some(&CaseOutcome::Passed),
                "{name} ran and passed over UDP:\n{report}"
            );
        }
        assert_eq!(device.armed(), 0, "every armed fault was used");
        assert!(device.reads() > 0, "the device served reads");
        assert_eq!(
            device.applied(),
            2,
            "one confirmed write and one whose confirmation was lost, each applied once"
        );
    }
}
