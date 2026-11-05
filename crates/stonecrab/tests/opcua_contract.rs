//! Public Driver/Connection contract against the first-party OPC UA wire responder.
#![cfg(all(feature = "opcua", feature = "testkit"))]
#![forbid(unsafe_code)]

use stonecrab as library;
#[cfg(test)]
#[path = "support/opcua_responder.rs"]
#[expect(
    dead_code,
    clippy::redundant_pub_crate,
    reason = "visibility is shared with private unit modules; other responder constructions are used by private lane/channel/read tests"
)]
mod responder;

#[cfg(test)]
mod tests {
    use super::responder::Device;
    use std::io::Write;
    use std::sync::Arc;
    use stonecrab::PlcValue;
    use stonecrab::opcua::OpcuaDriver;
    use stonecrab::testkit::{
        CaseOutcome, ContractFixture, ContractSuite, FakeClock, FakeTransportFactory,
        FixtureAddress, Script, Step, block_on_with,
    };

    #[test]
    fn opcua_public_connection_contract_suite_is_green() {
        let clock = FakeClock::default();
        let device = Device::default();
        let factory = FakeTransportFactory::new(clock.clone());
        for _ in 0..128 {
            factory.push_script(Script::strict(vec![Step::react(device.reactor())]));
        }
        let driver = OpcuaDriver::new()
            .with_clock(Arc::new(clock.clone()))
            .with_transport_factory(Arc::new(factory.clone()));
        let probe = factory.clone();
        let count = device.clone();
        let timeout = device.clone();
        let disconnect = device.clone();
        let exception = device.clone();
        let unknown = device.clone();
        let fixture = ContractFixture::new(
            Arc::new(clock.clone()),
            "ns=2;i=42",
            "ns=2;i=65535",
            Arc::new(FixtureAddress::new("elsewhere", None)),
            move || {
                u64::try_from(
                    probe.opens().len()
                        + probe
                            .probes()
                            .iter()
                            .map(|probe| probe.sent().len())
                            .sum::<usize>(),
                )
                .unwrap()
            },
            move || count.reads(),
        )
        .with_writable("ns=2;i=42;DINT", PlcValue::I32(1))
        .with_inject_unknown_after_transmit(move || unknown.unknown_after_transmit())
        .with_inject_timeout(move || timeout.timeout())
        .with_inject_disconnect(move || disconnect.disconnect())
        .with_inject_device_exception(move || exception.exception());
        let report = block_on_with(
            &clock,
            ContractSuite::run(
                &driver,
                "opcua://fake:4840/path?security-policy=NONE&request-timeout-ms=25&access=read-write",
                &fixture,
            ),
        )
        .unwrap();
        writeln!(std::io::stdout().lock(), "{report}").unwrap();
        assert!(report.is_green(), "{report}");
        assert_eq!(report.failed(), 0, "{report}");
        let mut skipped = Vec::new();
        for (case, outcome) in &report.cases {
            if let CaseOutcome::Skipped(reason) = outcome {
                skipped.push((case.name, reason.as_str()));
            }
        }
        assert_eq!(report.passed(), 24, "{report}");
        let expected = [
            "an unsupported write performs no I/O",
            "a supported ping answers and changes no state",
            "a ping to a silent device yields Timeout",
            "a cyclic subscription delivers one event per tick and discloses its interval",
            "a change-of-state subscription delivers a first value and then only changes",
            "a polling driver refuses an event subscription with no I/O",
            "unsubscribing stops the polling reads",
            "closing the connection ends its subscriptions after the queued events",
            "a stalled polling read skips the ticks that came due and never bursts",
        ];
        assert_eq!(
            skipped.iter().map(|(name, _)| *name).collect::<Vec<_>>(),
            expected,
            "{report}"
        );
        assert_eq!(
            report.passed() + report.skipped(),
            ContractSuite::cases().len()
        );
        assert!(skipped.iter().all(|(_, why)| !why.is_empty()), "{report}");
    }

    #[test]
    fn opcua_public_reconnect_preserves_the_contract_after_a_lost_read() {
        use stonecrab::{ConnectionState, Driver, ErrorKind, ReadRequest, Tag, TagId};
        let clock = FakeClock::default();
        let factory = FakeTransportFactory::new(clock.clone());
        let device = Device::default();
        for _ in 0..2 {
            factory.push_script(Script::strict(vec![Step::react(device.reactor())]));
        }
        let driver = OpcuaDriver::new()
            .with_clock(Arc::new(clock.clone()))
            .with_transport_factory(Arc::new(factory.clone()));
        let connection = block_on_with(
            &clock,
            driver.connect(
                driver
                    .parse_config("opcua://fake:4840/path?security-policy=NONE")
                    .unwrap(),
            ),
        )
        .unwrap();
        let request = || {
            ReadRequest::builder(connection.limits())
                .tag(Tag::new(
                    TagId::new("value").unwrap(),
                    driver
                        .parse_address("ns=2;i=42", &stonecrab::AddressOptions::default())
                        .unwrap(),
                ))
                .build()
                .unwrap()
        };
        device.disconnect();
        assert_eq!(
            block_on_with(&clock, connection.read(request()))
                .unwrap_err()
                .kind(),
            ErrorKind::Disconnected
        );
        assert!(matches!(
            connection.state(),
            ConnectionState::Reconnecting { attempt: 1, .. }
        ));
        assert_eq!(factory.opens().len(), 1);
        assert_eq!(
            block_on_with(&clock, connection.read(request()))
                .unwrap()
                .iter_ok()
                .count(),
            1
        );
        assert_eq!(factory.opens().len(), 2);
        assert_eq!(connection.state(), ConnectionState::Connected);
        block_on_with(&clock, connection.close()).unwrap();
    }
}
