//! The common driver contract suite (slice S13) run against `modbus-tcp` over real loopback
//! sockets (slice S26): the third fixture beside the fake-transport run (`contract.rs`,
//! S25b) and the pymodbus run (S27).
//!
//! `ModbusTcpDriver::new()` keeps its real clock (the Tokio `SystemClock`) and its real TCP
//! transport. Every transport it opens is a socket to a fresh testkit `ScriptedPeer` whose
//! script is one `Step::React` over the first-party MBAP responder
//! (`tests/support/responder.rs`) and the interop register map, so the device behind the
//! sockets is the one the fake-transport run uses.
//!
//! # Why a factory that spawns a peer
//! A `ScriptedPeer` accepts one connection and refuses the next, and the suite connects once
//! per case (and the driver reconnects after an injected disconnect). The testkit's
//! `ScriptedPeerFactory` is a `TransportFactory` that, on every open, spawns a new peer from
//! a script-building closure and delegates the connection to the real `TcpFactory`. No byte
//! passes through it, and the driver sees the same `TcpTransport` it sees in production.
//!
//! # Skips
//! The fixture's clock is the real `SystemClock`, so the suite's timing cases cannot run:
//! they need a fault that swallows a reply and a deterministic clock that moves to the
//! deadline, and loopback tests cover bytes only (decision D21). The fixture therefore has
//! no `inject_timeout` hook and the three cases that need it are skipped by name:
//!
//! - `an absent deadline uses the connection timeout`
//! - `a timeout carries diagnostics naming operation, state and category`
//! - `read retry stops after the configured maximum attempts`
//!
//! The silent-ping case needs the same hook and is skipped with them. The unsupported-write
//! case is skipped because the driver writes (S46) and the unsupported-ping case because it
//! supports ping (S47). The two write cases run: the read-only policy refuses with no I/O,
//! and a write whose confirmation is lost is `Unknown`. Over a real socket the loss is a
//! closed connection (`Reaction::Close` after the device applied the write), not a swallowed
//! reply, because a swallowed reply would wait out a real deadline. The disconnect,
//! device-exception and no-I/O cases run: the responder closes the peer or answers exception
//! 04 on demand.
#![cfg(all(feature = "modbus", feature = "testkit"))]
#![forbid(unsafe_code)]

#[cfg(test)]
#[path = "support/responder.rs"]
mod responder;

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::path::Path;
    use std::sync::Arc;

    use stonecrab::modbus::{ExceptionCode, ModbusTcpDriver, SystemClock};
    use stonecrab::testkit::{
        CaseOutcome, ContractFixture, ContractReport, ContractSuite, FixtureAddress, InteropMap,
        Reaction, Script, ScriptedPeerFactory, Step,
    };
    use stonecrab::{Clock, PlcValue};

    use crate::responder::Device;

    /// The connection string of the run, with writes granted (the suite needs it for a
    /// driver that supports write); the peer factory ignores the endpoint it names.
    const URI: &str = "modbus-tcp://127.0.0.1:5020?access=read-write";

    /// A readable holding register: logical 13 is the UINT 42424 of `MB-INTEROP-HR-04`.
    const READABLE: &str = "holding-register:13:UINT";

    /// An address that parses and lies far outside the 201-register block, so the device
    /// answers exception 02 (ILLEGAL DATA ADDRESS).
    const REJECTED: &str = "holding-register:60000:UINT";

    /// A holding register of the write area 100 to 200, which starts at zero.
    const WRITABLE: &str = "holding-register:100:UINT";

    const ABSENT_DEADLINE: &str = "an absent deadline uses the connection timeout";
    const TIMEOUT_DIAGNOSTICS: &str =
        "a timeout carries diagnostics naming operation, state and category";
    const RETRY_STOPS: &str = "read retry stops after the configured maximum attempts";
    const SILENT_PING: &str = "a ping to a silent device yields Timeout";
    const UNSUPPORTED_PING: &str = "an unsupported ping performs no I/O and changes no state";
    const SUPPORTED_PING: &str = "a supported ping answers and changes no state";
    const UNSUPPORTED_SUBSCRIBE: &str =
        "an unsupported subscribe performs no I/O and changes no state";
    const READ_ONLY_POLICY: &str = "a read-only policy rejects a write with no I/O";
    const UNKNOWN_NOT_REPLAYED: &str = "an unknown write outcome is never replayed";
    const UNSUPPORTED_WRITE: &str = "an unsupported write performs no I/O";

    /// The cases the fixture cannot run, by name, with the reason the report must give.
    const TIMING_SKIPS: [&str; 4] = [
        ABSENT_DEADLINE,
        TIMEOUT_DIAGNOSTICS,
        RETRY_STOPS,
        SILENT_PING,
    ];

    fn register_map() -> InteropMap {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/modbus/interop/register-map.toml");
        InteropMap::load(&path).expect("the interop register map loads")
    }

    /// The driver, the loopback world behind it and the fixture that wires the suite to it.
    struct Bench {
        farm: Arc<ScriptedPeerFactory>,
        device: Device,
        driver: ModbusTcpDriver,
        fixture: ContractFixture,
    }

    impl Bench {
        fn new() -> Self {
            let device = Device::from_map(&register_map());
            let script_device = device.clone();
            let farm = Arc::new(ScriptedPeerFactory::new(move || {
                Script::new(vec![Step::react(script_device.reactor())])
            }));
            let driver = ModbusTcpDriver::new()
                .with_transport_factory(Arc::<ScriptedPeerFactory>::clone(&farm));
            // What reached the peers: the opens plus the requests the device counted. A
            // request in flight is counted once the peer task has read it, so the probe is
            // exact for the "no I/O" cases, where nothing is sent at all.
            let probe_farm = Arc::clone(&farm);
            let probe_device = device.clone();
            let counter = device.clone();
            let exception = device.clone();
            let disconnect = device.clone();
            let unknown = device.clone();
            let clock: Arc<dyn Clock> = Arc::new(SystemClock::default());
            let fixture = ContractFixture::new(
                clock,
                READABLE,
                REJECTED,
                Arc::new(FixtureAddress::new("elsewhere", None)),
                move || {
                    probe_farm
                        .opens()
                        .saturating_add(u64::from(probe_device.reads()))
                },
                move || counter.reads(),
            )
            .with_writable(WRITABLE, PlcValue::U16(1))
            .with_inject_unknown_after_transmit(move || unknown.arm_apply_then_disconnect())
            .with_inject_disconnect(move || disconnect.arm_disconnect())
            .with_inject_device_exception(move || {
                exception.arm_exception(ExceptionCode::ServerDeviceFailure);
            });
            // No `inject_timeout`: the clock is real (see the module documentation).
            Self {
                farm,
                device,
                driver,
                fixture,
            }
        }

        async fn run(&self) -> ContractReport {
            let report = ContractSuite::run(&self.driver, URI, &self.fixture)
                .await
                .expect("the suite always resolves");
            writeln!(std::io::stdout().lock(), "{URI}\n{report}").expect("stdout");
            report
        }
    }

    // covers: SPEC test-layers / contract suite against modbus-tcp over loopback sockets
    #[tokio::test(flavor = "current_thread")]
    async fn the_suite_is_green_against_modbus_tcp_over_a_scripted_peer() {
        let bench = Bench::new();
        let report = bench.run().await;
        assert!(report.is_green(), "the suite is green:\n{report}");
        assert_eq!(report.failed(), 0, "no failures:\n{report}");
        assert_eq!(
            report.passed().saturating_add(report.skipped()),
            ContractSuite::cases().len(),
            "every case has an outcome:\n{report}"
        );
        assert_eq!(
            report.skipped(),
            6,
            "exactly the four timing skips, the unsupported-write skip and the \
             unsupported-ping skip:\n{report}"
        );
        assert_eq!(
            report.outcome(UNSUPPORTED_PING),
            Some(&CaseOutcome::Skipped(
                "driver reports ping supported; the case needs it unsupported".to_owned()
            )),
            "the driver supports ping:\n{report}"
        );
        assert_eq!(
            report.outcome(SUPPORTED_PING),
            Some(&CaseOutcome::Passed),
            "a ping over a real socket answers:\n{report}"
        );
        assert_eq!(
            report.outcome(UNSUPPORTED_SUBSCRIBE),
            Some(&CaseOutcome::Passed),
            "a subscribe is refused before any socket I/O:\n{report}"
        );
        for name in TIMING_SKIPS {
            assert_eq!(
                report.outcome(name),
                Some(&CaseOutcome::Skipped(
                    "fixture cannot inject timeout".to_owned()
                )),
                "{name} is skipped because the fixture cannot swallow a reply:\n{report}"
            );
        }
        assert_eq!(
            report.outcome(UNSUPPORTED_WRITE),
            Some(&CaseOutcome::Skipped(
                "driver reports write supported; the case needs it unsupported".to_owned()
            )),
            "the driver writes:\n{report}"
        );
        for name in [READ_ONLY_POLICY, UNKNOWN_NOT_REPLAYED] {
            assert_eq!(
                report.outcome(name),
                Some(&CaseOutcome::Passed),
                "{name} ran and passed over the sockets:\n{report}"
            );
        }
        for name in [
            "a disconnect during a read yields Disconnected",
            "a disconnect carries diagnostics naming operation, state and category",
            "a device exception carries diagnostics naming operation, state and category",
            "read retry skips non-retryable kinds",
            "a read correlates every tag with its result in request order",
            "a partial failure keeps the successes",
            "an elapsed deadline yields Timeout with no I/O",
            "a foreign address is rejected with no I/O",
        ] {
            assert_eq!(
                report.outcome(name),
                Some(&CaseOutcome::Passed),
                "{name} ran and passed:\n{report}"
            );
        }

        // Nothing is left armed, the device served the reads, and every peer the farm
        // spawned ran its script without a mismatch (a React-only script has none).
        assert_eq!(bench.device.armed(), 0, "every armed fault was used");
        assert!(bench.device.reads() > 0, "the device served reads");
        assert_eq!(
            bench.device.applied(),
            2,
            "one confirmed write (the capabilities case) and one unconfirmed (the closed \
             connection), each applied once and never replayed"
        );
        let reports = bench
            .farm
            .finish()
            .await
            .expect("peer tasks that did not panic");
        assert_eq!(
            u64::try_from(reports.len()).ok(),
            Some(bench.farm.opens()),
            "one report per transport the driver opened"
        );
        for peer in &reports {
            assert!(peer.mismatches.is_empty(), "{:?}", peer.mismatches);
            assert!(!peer.failed, "no peer script failed");
        }
        assert!(
            reports.iter().any(|peer| peer.closed),
            "the injected disconnect closed at least one peer"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn the_suite_reports_a_connection_string_the_driver_rejects_as_failures() {
        let bench = Bench::new();
        let report = ContractSuite::run(
            &bench.driver,
            "modbus-tcp:udp://127.0.0.1:5020",
            &bench.fixture,
        )
        .await
        .expect("the suite always resolves");
        assert_eq!(report.passed(), 0, "nothing can pass:\n{report}");
        assert!(report.failed() > 0, "failures are reported:\n{report}");
        assert_eq!(bench.farm.opens(), 0, "no peer was started");
    }

    #[test]
    fn the_responder_applies_writes_and_loses_a_confirmation_on_demand() {
        let device = Device::from_map(&register_map());
        let mut react = device.reactor();
        // Function 06 at wire address 99 (logical 100, the first register of the write
        // area): the echo repeats the request and the register holds the value.
        assert_eq!(
            react(&[0, 1, 0, 0, 0, 6, 1, 6, 0, 99, 0x12, 0x34]),
            Reaction::Reply(vec![0, 1, 0, 0, 0, 6, 1, 6, 0, 99, 0x12, 0x34])
        );
        assert_eq!(device.holding(100), Some(0x1234), "function 06 applied");
        // Function 16 writes two registers; the echo is the start and the quantity.
        assert_eq!(
            react(&[0, 2, 0, 0, 0, 11, 1, 0x10, 0, 100, 0, 2, 4, 0, 0x0A, 1, 2]),
            Reaction::Reply(vec![0, 2, 0, 0, 0, 6, 1, 0x10, 0, 100, 0, 2])
        );
        assert_eq!(
            (device.holding(101), device.holding(102)),
            (Some(0x000A), Some(0x0102)),
            "function 16 applied"
        );
        // Function 15 writes three coils from wire address 99: true, false, true.
        assert_eq!(
            react(&[0, 3, 0, 0, 0, 8, 1, 0x0F, 0, 99, 0, 3, 1, 0b101]),
            Reaction::Reply(vec![0, 3, 0, 0, 0, 6, 1, 0x0F, 0, 99, 0, 3])
        );
        assert_eq!(
            (device.coil(100), device.coil(101), device.coil(102)),
            (Some(true), Some(false), Some(true)),
            "function 15 applied"
        );
        assert_eq!(device.applied(), 3, "three writes applied");
        // A write past the block is ILLEGAL DATA ADDRESS and changes nothing.
        assert_eq!(
            react(&[0, 4, 0, 0, 0, 6, 1, 6, 0xEA, 0x60, 0, 1]),
            Reaction::Reply(vec![0, 4, 0, 0, 0, 3, 1, 0x86, 2])
        );
        assert_eq!(device.applied(), 3, "the refused write was not applied");
        // A lost confirmation: the register changes, the client hears nothing, or the
        // connection closes.
        device.arm_apply_then_swallow();
        assert_eq!(
            react(&[0, 5, 0, 0, 0, 6, 1, 6, 0, 99, 0, 7]),
            Reaction::Silence
        );
        assert_eq!(device.holding(100), Some(7), "applied, then silent");
        device.arm_apply_then_disconnect();
        assert_eq!(
            react(&[0, 6, 0, 0, 0, 6, 1, 6, 0, 99, 0, 8]),
            Reaction::Close
        );
        assert_eq!(device.holding(100), Some(8), "applied, then closed");
        assert_eq!(device.applied(), 5, "five writes applied");
        assert_eq!(device.armed(), 0, "both faults used");
    }

    #[test]
    fn each_fault_maps_to_its_reaction_and_is_used_once() {
        // The faults the loopback fixture does not arm (a swallowed reply) still work on the
        // shared responder; the unit-level check keeps its whole API exercised here.
        let device = Device::from_map(&register_map());
        let mut react = device.reactor();
        let request = [0, 1, 0, 0, 0, 6, 1, 3, 0, 12, 0, 1];
        device.arm_disconnect();
        device.arm_swallow();
        device.arm_exception(ExceptionCode::ServerDeviceFailure);
        assert_eq!(device.armed(), 3, "three faults queued");
        assert_eq!(react(&request), Reaction::Close, "a disconnect closes");
        assert_eq!(react(&request), Reaction::Silence, "a swallow is silent");
        assert_eq!(
            react(&request),
            Reaction::Reply(vec![0, 1, 0, 0, 0, 3, 1, 0x83, 4]),
            "an exception is a refusal"
        );
        assert_eq!(device.armed(), 0, "every fault was used once");
        assert_eq!(device.reads(), 3, "each request counted");
    }
}
