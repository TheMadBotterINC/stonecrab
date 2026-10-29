//! `ModbusTcpDriver::parse_config`: the connection-string grammar, the six upstream
//! parameters with their defaults and bounds, the rejection of unknown keys, and the way
//! the parsed options reach the connection (slice S26).
//!
//! The wire bytes are written out by hand from the MBAP layout and the PDU examples of the
//! Modbus Application Protocol Specification V1.1b; no byte comes from the codec under test.
//! Time is the fake clock (decision D21); nothing opens a socket.
#![cfg(all(feature = "modbus", feature = "testkit"))]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use proptest::prelude::*;
    use stonecrab::modbus::{ConnectionDefaults, ModbusOptions, ModbusTcpDriver, UnitId};
    use stonecrab::testkit::{FakeClock, FakeTransportFactory, Script, Step, block_on_with};
    use stonecrab::{
        AccessPolicy, AddressOptions, ByteOrder, Connection, ConnectionConfig, Driver, Endpoint,
        ErrorKind, PlcError, PlcValue, ReadRequest, ReadResponse, Tag, TagId, WordOrder,
    };

    const HOST: &str = "modbus-tcp://192.0.2.10";

    fn driver() -> ModbusTcpDriver {
        ModbusTcpDriver::new()
    }

    fn config(uri: &str) -> ConnectionConfig {
        driver()
            .parse_config(uri)
            .expect("a valid connection string")
    }

    fn options(uri: &str) -> ModbusOptions {
        config(uri)
            .driver_options_as::<ModbusOptions>()
            .expect("parse_config attaches the options")
            .clone()
    }

    fn refusal(uri: &str) -> PlcError {
        driver()
            .parse_config(uri)
            .expect_err("the connection string must be refused")
    }

    fn assert_refused(uri: &str) {
        assert_eq!(refusal(uri).kind(), ErrorKind::InvalidConfig, "{uri}");
    }

    fn tcp(host: &str, port: u16) -> Endpoint {
        Endpoint::Tcp {
            host: host.to_owned(),
            port,
        }
    }

    // covers: ALPHA_PLAN S26 / MB-TCP-TRANSPORTS tcp transport, default port 502
    #[test]
    fn driver_accepts_tcp_transport_and_defaults_port_502() {
        for uri in [
            "modbus-tcp://192.0.2.10",
            "modbus-tcp:tcp://192.0.2.10",
            "modbus-tcp://192.0.2.10?request-timeout-ms=100",
        ] {
            assert_eq!(config(uri).endpoint(), &tcp("192.0.2.10", 502), "{uri}");
        }
        assert_eq!(
            config("modbus-tcp:tcp://plc-1.line_2:1502").endpoint(),
            &tcp("plc-1.line_2", 1502)
        );
    }

    // covers: S61 / MB-TCP-TRANSPORTS-UDP, MB-QUIRK-A udp accepted with the declared port 502
    #[test]
    fn driver_accepts_the_udp_transport_and_defaults_port_502() {
        let custom = |text: &str| Endpoint::Custom(text.to_owned());
        assert_eq!(
            config("modbus-tcp:udp://192.0.2.10").endpoint(),
            &custom("192.0.2.10:502")
        );
        assert_eq!(
            config("modbus-tcp:udp://plc-1.line_2:1502?request-timeout-ms=100").endpoint(),
            &custom("plc-1.line_2:1502")
        );
        assert_eq!(
            config("modbus-tcp:udp://[::1]:5020").endpoint(),
            &custom("[::1]:5020")
        );
        let options = options("modbus-tcp:udp://192.0.2.10?default-unit-identifier=9");
        assert_eq!(
            options.unit_id,
            UnitId::from(9),
            "the parameters apply as for tcp"
        );
        for uri in [
            "modbus-tcp:udp://192.0.2.10:0",
            "modbus-tcp:udp://:502",
            "modbus-tcp:udp://::1",
            "modbus-tcp:udp://host/path",
        ] {
            assert_refused(uri);
        }
    }

    // covers: ALPHA_PLAN S26 / MB-TCP-TRANSPORTS, MB-QUIRK-A other transports refused
    #[test]
    fn driver_rejects_other_transport_codes() {
        for code in ["tls", "tls-psk", "test", "TCP", "UDP", "tcp2", "udp6", ""] {
            assert_refused(&format!("modbus-tcp:{code}://192.0.2.10"));
        }
        assert_refused("modbus-rtu://192.0.2.10");
        assert_refused("sim://bench");
        assert_refused("192.0.2.10");
        assert_refused("modbus-tcp:tls://192.0.2.10?request-timeout-ms=100");
    }

    // covers: ALPHA_PLAN S26 / MB-TCP-TRANSPORTS port bounds
    #[test]
    fn driver_rejects_port_zero_and_65536() {
        for port in [
            "0",
            "65536",
            "99999999999999999999",
            "http",
            "",
            "-1",
            "+5",
            "0x1F6",
        ] {
            assert_refused(&format!("modbus-tcp://192.0.2.10:{port}"));
        }
        assert_eq!(config("modbus-tcp://h:1").endpoint(), &tcp("h", 1));
        assert_eq!(config("modbus-tcp://h:65535").endpoint(), &tcp("h", 65535));
    }

    // covers: ALPHA_PLAN S26 / section 13.7 authority forms with parameters present
    #[test]
    fn a_bad_authority_is_refused_with_and_without_parameters() {
        for uri in [
            "modbus-tcp://",
            "modbus-tcp://?request-timeout-ms=100",
            "modbus-tcp://:502",
            "modbus-tcp://:502?request-timeout-ms=100",
            "modbus-tcp://[::1",
            "modbus-tcp://[::1]:0?request-timeout-ms=100",
            "modbus-tcp://[::1]x",
            "modbus-tcp://[]",
            "modbus-tcp://::1",
            "modbus-tcp://host/path?request-timeout-ms=100",
        ] {
            assert_refused(uri);
        }
        assert_eq!(
            config("modbus-tcp://[2001:db8::10]:1502?default-unit-identifier=3").endpoint(),
            &tcp("2001:db8::10", 1502)
        );
        assert_eq!(
            config("modbus-tcp://[::1]?default-unit-identifier=3").endpoint(),
            &tcp("::1", 502)
        );
    }

    // covers: ALPHA_PLAN S26 / MB-CFG-REQUEST-TIMEOUT default
    #[test]
    fn config_request_timeout_default_is_5000() {
        let config = config(HOST);
        assert_eq!(
            config
                .driver_options_as::<ModbusOptions>()
                .map(|o| o.request_timeout),
            Some(Duration::from_millis(5000))
        );
        assert_eq!(config.timeouts().request, Duration::from_millis(5000));
    }

    // covers: ALPHA_PLAN S26 / MB-CFG-REQUEST-TIMEOUT bounds
    #[test]
    fn config_request_timeout_rejects_zero_negative_and_non_numeric() {
        for value in [
            "0", "-1", "-0", "+5", "", "abc", "1.5", "1e3", "5s", " 5", "5 ",
        ] {
            assert_refused(&format!("{HOST}?request-timeout-ms={value}"));
        }
        let accepted = config(&format!("{HOST}?request-timeout-ms=800"));
        assert_eq!(
            accepted
                .driver_options_as::<ModbusOptions>()
                .map(|o| o.request_timeout),
            Some(Duration::from_millis(800))
        );
        assert_eq!(
            accepted.timeouts().request,
            Duration::from_millis(800),
            "the connection enforces Timeouts.request"
        );
        assert_eq!(
            accepted.timeouts().connect,
            Duration::from_secs(5),
            "the connect timeout keeps the core default"
        );
    }

    // covers: ALPHA_PLAN S26 / MB-CFG-UNIT-ID default
    #[test]
    fn config_unit_id_default_is_1() {
        assert_eq!(options(HOST).unit_id, UnitId::from(1));
    }

    // covers: ALPHA_PLAN S26 / MB-CFG-UNIT-ID divergence, bounded to one byte
    #[test]
    fn config_unit_id_rejects_256_and_negative() {
        for value in [
            "256",
            "-1",
            "1000",
            "99999999999",
            "",
            "abc",
            "+1",
            "0x10",
            "1.0",
        ] {
            assert_refused(&format!("{HOST}?default-unit-identifier={value}"));
        }
    }

    // covers: ALPHA_PLAN S26 / MB-CFG-UNIT-ID boundaries accepted
    #[test]
    fn config_unit_id_accepts_0_and_255() {
        for (value, expected) in [("0", 0), ("1", 1), ("247", 247), ("255", 255)] {
            assert_eq!(
                options(&format!("{HOST}?default-unit-identifier={value}")).unit_id,
                UnitId::from(expected),
                "{value}"
            );
        }
    }

    // covers: ALPHA_PLAN S26 / MB-CFG-PING-ADDRESS default
    #[test]
    fn config_ping_address_default_parses() {
        let plc4x = AddressOptions::builder()
            .convention("plc4x")
            .build()
            .expect("options");
        let expected = stonecrab::modbus::ModbusAddress::parse("4x00001:BOOL", &plc4x)
            .expect("the upstream default text");
        assert_eq!(options(HOST).ping_address, expected);
        assert_eq!(
            options(&format!("{HOST}?ping-address=4x00001:BOOL")).ping_address,
            expected
        );
    }

    // covers: ALPHA_PLAN S26 / MB-CFG-PING-ADDRESS validated whatever the connection convention
    #[test]
    fn config_ping_address_invalid_is_rejected() {
        for value in [
            "",
            "nonsense",
            "holding-register:0",
            "coil:1:INT",
            "4x:INT",
            "99999",
        ] {
            assert_refused(&format!("{HOST}?ping-address={value}"));
        }
        // The bare short form is read under plc4x even when the connection names no
        // convention: the ping address has its own, fixed one.
        let bare = options(&format!("{HOST}?ping-address=40007:UINT"));
        let long = options(&format!("{HOST}?ping-address=holding-register:7:UINT"));
        assert_eq!(bare.ping_address, long.ping_address);
        assert_eq!(bare.ping_address.logical_address(), 7);
    }

    // covers: MB-CFG-PING-ADDRESS / connecting sends no ping, whatever the ping address
    #[test]
    fn connecting_sends_no_ping_whatever_the_ping_address() {
        let clock = FakeClock::default();
        let factory = FakeTransportFactory::new(clock.clone());
        factory.push_script(Script::strict(vec![]));
        let driver = ModbusTcpDriver::new()
            .with_clock(Arc::new(clock.clone()))
            .with_transport_factory(Arc::new(factory.clone()));
        let config = driver
            .parse_config(&format!("{HOST}?ping-address=holding-register:9:UINT"))
            .expect("config");
        let connection = block_on_with(&clock, driver.connect(config)).expect("connect");
        let probes = factory.probes();
        assert_eq!(probes.len(), 1, "one transport");
        assert!(
            probes.first().expect("probe").sent().is_empty(),
            "connecting sends no ping"
        );
        assert!(
            connection
                .capabilities()
                .supports(stonecrab::Operation::Subscribe),
            "subscriptions poll (S55) and the ping address does not change that"
        );
        block_on_with(&clock, connection.close()).expect("close");
    }

    // covers: ALPHA_PLAN S26 / MB-CFG-BYTE-ORDER default
    #[test]
    fn config_byte_order_default_is_big_endian() {
        let options = options(HOST);
        assert_eq!(
            (options.byte_order, options.word_order),
            (ByteOrder::BigEndian, WordOrder::HighWordFirst)
        );
    }

    // covers: ALPHA_PLAN S26 / MB-CFG-BYTE-ORDER the four names map per ADR-0006
    #[test]
    fn config_byte_order_accepts_four_upstream_names() {
        for (name, expected) in [
            (
                "BIG_ENDIAN",
                (ByteOrder::BigEndian, WordOrder::HighWordFirst),
            ),
            (
                "LITTLE_ENDIAN",
                (ByteOrder::LittleEndian, WordOrder::LowWordFirst),
            ),
            (
                "BIG_ENDIAN_BYTE_SWAP",
                (ByteOrder::LittleEndian, WordOrder::HighWordFirst),
            ),
            (
                "LITTLE_ENDIAN_BYTE_SWAP",
                (ByteOrder::BigEndian, WordOrder::LowWordFirst),
            ),
        ] {
            let read = options(&format!("{HOST}?default-payload-byte-order={name}"));
            assert_eq!((read.byte_order, read.word_order), expected, "{name}");
        }
    }

    // covers: ALPHA_PLAN S26 / MB-CFG-BYTE-ORDER unknown names refused
    #[test]
    fn config_byte_order_unknown_name_is_rejected() {
        for name in [
            "",
            "big_endian",
            "MIDDLE_ENDIAN",
            "BIG_ENDIAN ",
            "1",
            "BIG_ENDIAN_SWAP",
        ] {
            assert_refused(&format!("{HOST}?default-payload-byte-order={name}"));
        }
    }

    // covers: ALPHA_PLAN S26 / MB-CFG-MAX-COILS default
    #[test]
    fn config_max_coils_default_is_2000() {
        assert_eq!(options(HOST).max_coils.get(), 2000);
    }

    // covers: ALPHA_PLAN S26 / MB-CFG-MAX-COILS bounds 1..=2000
    #[test]
    fn config_max_coils_rejects_zero_and_above_2000() {
        for value in ["0", "2001", "65536", "-1", "", "abc", "+1", "1.5"] {
            assert_refused(&format!("{HOST}?max-coils-per-request={value}"));
        }
        assert_eq!(
            options(&format!("{HOST}?max-coils-per-request=1"))
                .max_coils
                .get(),
            1
        );
        assert_eq!(
            options(&format!("{HOST}?max-coils-per-request=2000"))
                .max_coils
                .get(),
            2000
        );
    }

    // covers: ALPHA_PLAN S26 / MB-CFG-MAX-REGISTERS default
    #[test]
    fn config_max_registers_default_is_125() {
        assert_eq!(options(HOST).max_registers.get(), 125);
    }

    // covers: ALPHA_PLAN S26 / MB-CFG-MAX-REGISTERS bounds 1..=125
    #[test]
    fn config_max_registers_rejects_zero_and_above_125() {
        for value in ["0", "126", "65536", "-1", "", "abc", "+1", "1.5"] {
            assert_refused(&format!("{HOST}?max-registers-per-request={value}"));
        }
        assert_eq!(
            options(&format!("{HOST}?max-registers-per-request=1"))
                .max_registers
                .get(),
            1
        );
        assert_eq!(
            options(&format!("{HOST}?max-registers-per-request=125"))
                .max_registers
                .get(),
            125
        );
    }

    // covers: MB-CFG-ACCESS / writes are granted by the connection string, read-only otherwise
    #[test]
    fn config_access_defaults_to_read_only_and_read_write_grants_writes() {
        assert_eq!(config(HOST).access(), AccessPolicy::ReadOnly, "the default");
        assert_eq!(
            config(&format!("{HOST}?access=read-only")).access(),
            AccessPolicy::ReadOnly
        );
        let granted = config(&format!("{HOST}?access=read-write&request-timeout-ms=800"));
        assert_eq!(granted.access(), AccessPolicy::ReadWrite);
        assert_eq!(
            granted
                .driver_options_as::<ModbusOptions>()
                .expect("options")
                .request_timeout,
            Duration::from_millis(800),
            "the other parameters are read beside it"
        );
        for bad in [
            "",
            "write",
            "rw",
            "READ-WRITE",
            "true",
            "read-write,read-only",
        ] {
            let error = refusal(&format!("{HOST}?access={bad}"));
            assert_eq!(error.kind(), ErrorKind::InvalidConfig, "{bad:?}");
            assert!(
                error.to_string().contains("access"),
                "names the key: {error}"
            );
        }
        // The key is spelled as the AccessPolicy displays it, and no other spelling counts.
        assert_refused(&format!("{HOST}?write=true"));
    }

    // covers: ALPHA_PLAN S26 / MB-CFG-UNKNOWN divergence
    #[test]
    fn config_unknown_key_is_rejected() {
        for key in [
            "surprise",
            "allow-unsupported-transport",
            "tcp.keep-alive",
            "udp.local-port",
            "udp.max-packet-size",
            "udp.multicast-ttl",
            "udp.read-timeout-ms",
            "REQUEST-TIMEOUT-MS",
            "request-timeout",
            "unit-id",
        ] {
            let error = refusal(&format!("{HOST}?{key}=1"));
            assert_eq!(error.kind(), ErrorKind::InvalidConfig, "{key}");
            assert!(error.to_string().contains(key), "names the key: {error}");
        }
        // A known key beside an unknown one does not rescue the string.
        assert_refused(&format!("{HOST}?request-timeout-ms=100&surprise=1"));
        assert_refused(&format!("{HOST}?surprise=1&request-timeout-ms=100"));
    }

    // covers: ALPHA_PLAN S26 / MB-QUIRK-F the misspelt unit-identifier of the upstream suite
    #[test]
    fn config_misspelt_unit_identifier_is_rejected() {
        let error = refusal("modbus-tcp://127.0.0.1:5020?unit-identifier=1");
        assert_eq!(error.kind(), ErrorKind::InvalidConfig);
        assert!(
            error.to_string().contains("default-unit-identifier"),
            "the message names the real key: {error}"
        );
    }

    // covers: ALPHA_PLAN S26 / MB-CFG-UNKNOWN duplicate keys are refused by the core grammar
    #[test]
    fn config_duplicate_key_is_rejected() {
        assert_refused(&format!(
            "{HOST}?default-unit-identifier=1&default-unit-identifier=1"
        ));
        assert_refused(&format!(
            "{HOST}?request-timeout-ms=100&request-timeout-ms=200"
        ));
        assert_refused(&format!(
            "{HOST}?address-convention=plc4x&address-convention=plc4x"
        ));
    }

    // covers: ALPHA_PLAN S26 / section 13.7 empty and malformed parameter lists
    #[test]
    fn malformed_parameter_lists_are_refused() {
        for suffix in [
            "?",
            "?&",
            "?=1",
            "?request-timeout-ms",
            "?a=1&&b=2",
            "?a=1&",
            "?%41=1",
        ] {
            assert_refused(&format!("{HOST}{suffix}"));
        }
    }

    // covers: ALPHA_PLAN S26 / section 13.7 each parameter at min-1, max+1, non-numeric, empty
    #[test]
    fn every_numeric_parameter_refuses_its_edges() {
        for (key, below, above) in [
            ("request-timeout-ms", "0", "99999999999999999999999"),
            ("default-unit-identifier", "-1", "256"),
            ("max-coils-per-request", "0", "2001"),
            ("max-registers-per-request", "0", "126"),
        ] {
            for value in [below, above, "x", ""] {
                assert_refused(&format!("{HOST}?{key}={value}"));
            }
        }
    }

    // covers: ALPHA_PLAN S26 / section 13.7 a refused value names the rule it broke
    #[test]
    fn text_that_is_not_digits_names_the_digits_rule_and_a_huge_number_the_bounds() {
        for key in [
            "default-unit-identifier",
            "max-coils-per-request",
            "max-registers-per-request",
        ] {
            for value in ["", "abc", "+1", "-1", "1.5", "12a"] {
                let text = refusal(&format!("{HOST}?{key}={value}")).to_string();
                assert!(
                    text.contains("whole number in decimal digits"),
                    "{key}={value:?} should name the digits rule: {text}"
                );
            }
            // Only digits, but too large even for u64: the bounds message applies.
            let text = refusal(&format!("{HOST}?{key}=99999999999999999999999")).to_string();
            assert!(
                text.contains("must be a whole number from"),
                "{key} huge value should name the bounds: {text}"
            );
        }
    }

    // covers: ALPHA_PLAN S26 / address-convention mapped into the AddressOptions
    #[test]
    fn address_convention_plc4x_reaches_the_address_options() {
        let plain = config(HOST);
        assert_eq!(plain.address_options().convention(), None);
        let explicit = config(&format!("{HOST}?address-convention=plc4x"));
        assert_eq!(explicit.address_options().convention(), Some("plc4x"));
        let driver = driver();
        assert!(
            driver
                .parse_address("40001", &plain.address_options())
                .is_err()
        );
        let address = driver
            .parse_address("40001:UINT", &explicit.address_options())
            .expect("the bare form under the explicit convention");
        assert_eq!(address.canonical(), "holding-register:1:UINT");
        for value in ["", "zero_based", "one-based", "PLC4X", "plc4x2"] {
            assert_refused(&format!("{HOST}?address-convention={value}"));
        }
    }

    // covers: MB-ADDR-CONVENTIONS
    #[test]
    fn address_convention_names_reach_the_address_options() {
        let driver = driver();
        for (name, canonical) in [
            ("plc4x", "holding-register:1:UINT"),
            ("one-based-40001", "holding-register:1:UINT"),
            ("zero-based", "holding-register:2:UINT"),
        ] {
            let config = config(&format!("{HOST}?address-convention={name}"));
            assert_eq!(config.address_options().convention(), Some(name), "{name}");
            let address = driver
                .parse_address("40001:UINT", &config.address_options())
                .expect("the bare form under an explicit convention");
            assert_eq!(address.canonical(), canonical, "{name}");
        }
    }

    // covers: ALPHA_PLAN S26 / error messages never echo the connection string
    #[test]
    fn a_refusal_never_repeats_the_connection_string() {
        for uri in [
            "modbus-tcp://secret-host-name:0",
            "modbus-tcp://secret-host-name?default-unit-identifier=999",
            "modbus-tcp://secret-host-name?ping-address=secret-address",
            "modbus-tcp://secret-host-name?secret-key=secret-value",
            "modbus-tcp://secret-host-name?request-timeout-ms=secret-value",
            "modbus-tcp://secret-host-name?default-payload-byte-order=secret-value",
            "modbus-tcp://secret-host-name?max-coils-per-request=secret-value",
            "modbus-tcp:secret-transport://secret-host-name",
        ] {
            let text = refusal(uri).to_string();
            for secret in [
                "secret-host-name",
                "secret-value",
                "secret-address",
                "secret-transport",
            ] {
                assert!(!text.contains(secret), "{uri} leaked {secret}: {text}");
            }
        }
    }

    // covers: ALPHA_PLAN S26 / a full string carries every parameter at once
    #[test]
    fn a_string_with_every_parameter_is_read_whole() {
        let config = config(
            "modbus-tcp:tcp://[::1]:5020?request-timeout-ms=750&default-unit-identifier=17\
             &ping-address=3x00005:UINT&default-payload-byte-order=LITTLE_ENDIAN_BYTE_SWAP\
             &max-coils-per-request=64&max-registers-per-request=20&address-convention=plc4x",
        );
        assert_eq!(config.endpoint(), &tcp("::1", 5020));
        assert_eq!(config.address_options().convention(), Some("plc4x"));
        let options = config
            .driver_options_as::<ModbusOptions>()
            .expect("options");
        assert_eq!(options.request_timeout, Duration::from_millis(750));
        assert_eq!(options.unit_id, UnitId::from(17));
        assert_eq!(options.ping_address.logical_address(), 5);
        assert_eq!(
            (options.byte_order, options.word_order),
            (ByteOrder::BigEndian, WordOrder::LowWordFirst)
        );
        assert_eq!(
            (options.max_coils.get(), options.max_registers.get()),
            (64, 20)
        );
        let text = config.redacted_display();
        assert!(text.contains("options=ModbusOptions"), "{text}");
    }

    // ---- the options reach the connection ----

    struct Rig {
        clock: FakeClock,
        factory: FakeTransportFactory,
        connection: Box<dyn Connection>,
        driver: ModbusTcpDriver,
        config_options: AddressOptions,
    }

    impl Rig {
        fn open(
            driver: ModbusTcpDriver,
            config: ConnectionConfig,
            clock: &FakeClock,
            factory: &FakeTransportFactory,
        ) -> Self {
            let config_options = config.address_options();
            let connection = block_on_with(clock, driver.connect(config)).expect("connect");
            Self {
                clock: clock.clone(),
                factory: factory.clone(),
                connection,
                driver,
                config_options,
            }
        }

        fn tag(&self, id: &str, text: &str) -> Tag {
            let address = self
                .driver
                .parse_address(text, &self.config_options)
                .expect("address");
            Tag::new(TagId::new(id).expect("tag id"), address)
        }

        fn read(&self, tags: Vec<Tag>) -> ReadResponse {
            let mut builder = ReadRequest::builder(self.connection.limits());
            for tag in tags {
                builder = builder.tag(tag);
            }
            let request = builder.build().expect("request");
            block_on_with(&self.clock, self.connection.read(request)).expect("read")
        }

        fn sent(&self) -> Vec<Vec<u8>> {
            self.factory
                .probes()
                .first()
                .expect("a transport")
                .sent()
                .iter()
                .map(|frame| frame.to_vec())
                .collect()
        }
    }

    fn fake_driver(steps: Vec<Step>) -> (ModbusTcpDriver, FakeClock, FakeTransportFactory) {
        let clock = FakeClock::default();
        let factory = FakeTransportFactory::new(clock.clone());
        factory.push_script(Script::strict(steps));
        let driver = ModbusTcpDriver::new()
            .with_clock(Arc::new(clock.clone()))
            .with_transport_factory(Arc::new(factory.clone()));
        (driver, clock, factory)
    }

    fn value(response: &ReadResponse, id: &str) -> PlcValue {
        response
            .get(&TagId::new(id).expect("tag id"))
            .expect("the item")
            .outcome()
            .as_ref()
            .expect("a value")
            .clone()
    }

    /// FC03, transaction 1, the given unit, wire address 0, one register.
    fn one_register_request(unit: u8) -> Vec<u8> {
        vec![0, 1, 0, 0, 0, 6, unit, 3, 0, 0, 0, 1]
    }

    // covers: ALPHA_PLAN S26 / MB-CFG-UNIT-ID the parsed unit is the one on the wire
    #[test]
    fn the_parsed_unit_identifier_addresses_the_request() {
        let (driver, clock, factory) = fake_driver(vec![
            Step::Expect(one_register_request(17)),
            Step::Reply(vec![0, 1, 0, 0, 0, 5, 17, 3, 2, 0, 42]),
        ]);
        let config = driver
            .parse_config(&format!("{HOST}?default-unit-identifier=17"))
            .expect("config");
        let rig = Rig::open(driver, config, &clock, &factory);
        let response = rig.read(vec![rig.tag("a", "holding-register:1:UINT")]);
        assert_eq!(value(&response, "a"), PlcValue::U16(42));
        assert_eq!(rig.sent(), vec![one_register_request(17)]);
    }

    // covers: ALPHA_PLAN S26 / MB-CFG-BYTE-ORDER the parsed order decodes the reply
    #[test]
    fn the_parsed_byte_order_decodes_the_reply() {
        // Registers 0x0001 0x0002 are the wire bytes 00 01 00 02: DWORD 0x00010002 under
        // BIG_ENDIAN, and its full reversal 0x02000100 under LITTLE_ENDIAN ([4,3,2,1]).
        for (name, expected) in [
            ("BIG_ENDIAN", 0x0001_0002_u32),
            ("LITTLE_ENDIAN", 0x0200_0100),
        ] {
            let (driver, clock, factory) = fake_driver(vec![
                Step::Expect(vec![0, 1, 0, 0, 0, 6, 1, 3, 0, 0, 0, 2]),
                Step::Reply(vec![0, 1, 0, 0, 0, 7, 1, 3, 4, 0, 1, 0, 2]),
            ]);
            let config = driver
                .parse_config(&format!("{HOST}?default-payload-byte-order={name}"))
                .expect("config");
            let rig = Rig::open(driver, config, &clock, &factory);
            let response = rig.read(vec![rig.tag("a", "holding-register:1:DWORD")]);
            assert_eq!(value(&response, "a"), PlcValue::U32(expected), "{name}");
        }
    }

    // covers: ALPHA_PLAN S26 / MB-CFG-MAX-REGISTERS the parsed limit chunks the plan
    #[test]
    fn the_parsed_register_limit_splits_the_read_into_chunks() {
        let (driver, clock, factory) = fake_driver(vec![
            Step::Expect(vec![0, 1, 0, 0, 0, 6, 1, 3, 0, 0, 0, 2]),
            Step::Reply(vec![0, 1, 0, 0, 0, 7, 1, 3, 4, 0, 1, 0, 2]),
            Step::Expect(vec![0, 2, 0, 0, 0, 6, 1, 3, 0, 2, 0, 1]),
            Step::Reply(vec![0, 2, 0, 0, 0, 5, 1, 3, 2, 0, 3]),
        ]);
        let config = driver
            .parse_config(&format!("{HOST}?max-registers-per-request=2"))
            .expect("config");
        let rig = Rig::open(driver, config, &clock, &factory);
        let response = rig.read(vec![
            rig.tag("a", "holding-register:1:UINT"),
            rig.tag("b", "holding-register:2:UINT"),
            rig.tag("c", "holding-register:3:UINT"),
        ]);
        assert_eq!(
            response.split_into().get(),
            2,
            "three registers, two per request"
        );
        assert_eq!(value(&response, "a"), PlcValue::U16(1));
        assert_eq!(value(&response, "b"), PlcValue::U16(2));
        assert_eq!(value(&response, "c"), PlcValue::U16(3));
    }

    // covers: ALPHA_PLAN S26 / MB-CFG-MAX-COILS the parsed limit chunks the plan
    #[test]
    fn the_parsed_coil_limit_splits_the_read_into_chunks() {
        let (driver, clock, factory) = fake_driver(vec![
            Step::Expect(vec![0, 1, 0, 0, 0, 6, 1, 1, 0, 0, 0, 2]),
            Step::Reply(vec![0, 1, 0, 0, 0, 4, 1, 1, 1, 0b01]),
            Step::Expect(vec![0, 2, 0, 0, 0, 6, 1, 1, 0, 2, 0, 1]),
            Step::Reply(vec![0, 2, 0, 0, 0, 4, 1, 1, 1, 0b01]),
        ]);
        let config = driver
            .parse_config(&format!("{HOST}?max-coils-per-request=2"))
            .expect("config");
        let rig = Rig::open(driver, config, &clock, &factory);
        let response = rig.read(vec![
            rig.tag("a", "coil:1"),
            rig.tag("b", "coil:2"),
            rig.tag("c", "coil:3"),
        ]);
        assert_eq!(response.split_into().get(), 2);
        assert_eq!(value(&response, "a"), PlcValue::Bool(true));
        assert_eq!(value(&response, "b"), PlcValue::Bool(false));
        assert_eq!(value(&response, "c"), PlcValue::Bool(true));
    }

    // covers: ALPHA_PLAN S26 / a configuration built by hand carries no options and still connects
    #[test]
    fn a_hand_built_configuration_connects_with_the_driver_defaults() {
        let (driver, clock, factory) = fake_driver(vec![
            Step::Expect(one_register_request(9)),
            Step::Reply(vec![0, 1, 0, 0, 0, 5, 9, 3, 2, 0, 7]),
        ]);
        let mut defaults = ConnectionDefaults::default();
        defaults.unit_id = UnitId::from(9);
        let driver = driver.with_defaults(defaults);
        let config = ConnectionConfig::builder("modbus-tcp", tcp("192.0.2.10", 502))
            .build()
            .expect("config");
        assert!(config.driver_options().is_none());
        let rig = Rig::open(driver, config, &clock, &factory);
        let response = rig.read(vec![rig.tag("a", "holding-register:1:UINT")]);
        assert_eq!(value(&response, "a"), PlcValue::U16(7));
    }

    // covers: ALPHA_PLAN S26 / options of another driver are ignored, not misread
    #[test]
    fn options_of_another_type_fall_back_to_the_driver_defaults() {
        #[derive(Debug)]
        struct Foreign;
        impl stonecrab::DriverOptions for Foreign {}
        let (driver, clock, factory) = fake_driver(vec![
            Step::Expect(one_register_request(1)),
            Step::Reply(vec![0, 1, 0, 0, 0, 5, 1, 3, 2, 0, 7]),
        ]);
        let config = ConnectionConfig::builder("modbus-tcp", tcp("192.0.2.10", 502))
            .driver_options(Arc::new(Foreign))
            .build()
            .expect("config");
        let rig = Rig::open(driver, config, &clock, &factory);
        let response = rig.read(vec![rig.tag("a", "holding-register:1:UINT")]);
        assert_eq!(value(&response, "a"), PlcValue::U16(7));
    }

    // covers: ALPHA_PLAN S26 / hand-built defaults above the protocol maxima are refused at connect
    #[test]
    fn connect_refuses_request_limits_above_the_protocol_maxima() {
        let big = std::num::NonZeroU16::new(126).expect("non-zero");
        for registers in [true, false] {
            let (driver, clock, factory) = fake_driver(vec![]);
            let mut defaults = ConnectionDefaults::default();
            if registers {
                defaults.max_registers = big;
            } else {
                defaults.max_coils = std::num::NonZeroU16::new(2001).expect("non-zero");
            }
            let driver = driver.with_defaults(defaults);
            let config = ConnectionConfig::builder("modbus-tcp", tcp("192.0.2.10", 502))
                .build()
                .expect("config");
            let error = block_on_with(&clock, driver.connect(config))
                .err()
                .expect("limits above the protocol maxima");
            assert_eq!(error.kind(), ErrorKind::InvalidConfig);
            assert!(factory.opens().is_empty(), "nothing was opened");
        }
    }

    // ---- properties ----

    fn well_formed() -> impl Strategy<Value = String> {
        let host = prop_oneof![
            Just("192.0.2.10".to_owned()),
            Just("plc-1.line_2".to_owned()),
            Just("[::1]".to_owned()),
            "[a-z0-9._-]{0,12}",
        ];
        let port = prop_oneof![
            Just(String::new()),
            (0_u32..70_000).prop_map(|port| format!(":{port}")),
            "[0-9a-z+-]{1,6}".prop_map(|port| format!(":{port}")),
        ];
        let key = prop_oneof![
            Just("request-timeout-ms".to_owned()),
            Just("default-unit-identifier".to_owned()),
            Just("ping-address".to_owned()),
            Just("default-payload-byte-order".to_owned()),
            Just("max-coils-per-request".to_owned()),
            Just("max-registers-per-request".to_owned()),
            Just("address-convention".to_owned()),
            "[a-z-]{1,12}",
        ];
        let value = prop_oneof![
            (0_u64..100_000).prop_map(|number| number.to_string()),
            Just("LITTLE_ENDIAN".to_owned()),
            Just("plc4x".to_owned()),
            Just("holding-register:5:UINT".to_owned()),
            "[ -~]{0,16}",
        ];
        (host, port, proptest::collection::vec((key, value), 0..5)).prop_map(
            |(host, port, parameters)| {
                let query = parameters
                    .iter()
                    .map(|(key, value)| format!("{key}={value}"))
                    .collect::<Vec<_>>()
                    .join("&");
                if query.is_empty() {
                    format!("modbus-tcp://{host}{port}")
                } else {
                    format!("modbus-tcp://{host}{port}?{query}")
                }
            },
        )
    }

    proptest! {
        // covers: ALPHA_PLAN S26 / prop_parse_config_never_panics (arbitrary text)
        #[test]
        fn prop_parse_config_never_panics_on_arbitrary_text(text in "\\PC{0,200}") {
            drop(driver().parse_config(&text));
        }

        // covers: ALPHA_PLAN S26 / prop_parse_config_never_panics (well-formed with random parameters)
        #[test]
        fn prop_parse_config_never_panics(uri in well_formed()) {
            match driver().parse_config(&uri) {
                Ok(config) => {
                    let options = config.driver_options_as::<ModbusOptions>().expect("options");
                    prop_assert!(options.request_timeout > Duration::ZERO);
                    prop_assert!((1..=125).contains(&options.max_registers.get()));
                    prop_assert!((1..=2000).contains(&options.max_coils.get()));
                    prop_assert_eq!(config.timeouts().request, options.request_timeout);
                    prop_assert_eq!(config.scheme(), "modbus-tcp");
                }
                Err(error) => prop_assert_eq!(error.kind(), ErrorKind::InvalidConfig),
            }
        }
    }

    // covers: CONTRIBUTING "Fuzz findings" / regression for fuzz artifact crash-edad0273 (hosted run
    // 40537329063): the input equals a core error message, so the fuzz harness's "never echoes
    // the connection string" check fired on a fixed template; the library behaviour was and is
    // a clean InvalidConfig. The input is kept as the corpus seed `regression_message_text`.
    #[test]
    fn fuzz_modbus_connection_string_edad0273() {
        let text = "connection string contains a control character or whitespace";
        let error = ModbusTcpDriver::new()
            .parse_config(text)
            .expect_err("not a connection string");
        assert_eq!(error.kind(), ErrorKind::InvalidConfig, "kind");
        let message = error.to_string();
        assert!(message.is_ascii(), "ASCII template: {message}");
        assert!(
            !message.chars().any(char::is_control),
            "no control characters: {message}"
        );
    }

    // covers: CONTRIBUTING "Fuzz findings" / regression for fuzz artifact crash-d69b53f6: an
    // unknown key with non-ASCII text is named in the message (shortened), which the first
    // harness wrongly treated as an echo of the connection string; the authority is not
    // repeated. The input is kept as the corpus seed `regression_unknown_key_non_ascii`.
    #[test]
    fn fuzz_modbus_connection_string_d69b53f6() {
        let text = "modbus-tcp://0?tcw\u{3e4}/@9c=a";
        let error = ModbusTcpDriver::new()
            .parse_config(text)
            .expect_err("an unknown key");
        assert_eq!(error.kind(), ErrorKind::InvalidConfig, "kind");
        let message = error.to_string();
        assert!(
            message.contains("unknown connection parameter"),
            "names the problem: {message}"
        );
        assert!(
            !message.chars().any(char::is_control),
            "no control characters: {message}"
        );
        assert!(
            !message.contains("//0?"),
            "the authority is not repeated: {message}"
        );
    }

    // covers: CONTRIBUTING "Fuzz findings" / regression for fuzz artifact crash-fa68d98a: a value
    // that also occurs inside an unknown key surfaces through the key echo, which the harness
    // wrongly read as a value echo; the library names the key and nothing else. The input is
    // kept as the corpus seed `regression_value_inside_key`.
    #[test]
    fn fuzz_modbus_connection_string_fa68d98a() {
        let text = "modbus-tcp://10.92..110?pinder-agdsso\u{4e8}\u{4e8}=o\u{4e8}";
        let error = ModbusTcpDriver::new()
            .parse_config(text)
            .expect_err("an unknown key");
        assert_eq!(error.kind(), ErrorKind::InvalidConfig, "kind");
        let message = error.to_string();
        assert!(
            message.contains("unknown connection parameter"),
            "names the problem: {message}"
        );
        assert!(
            !message.contains("10.92..110"),
            "the authority is not repeated: {message}"
        );
    }
}
