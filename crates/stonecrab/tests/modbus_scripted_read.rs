//! A holding-register read over a real loopback socket (slice S26).
//!
//! `ModbusTcpDriver::new()` is registered in a `DriverRegistry`, connects with
//! `registry.connect("modbus-tcp://127.0.0.1:<port>")` to a testkit `ScriptedPeer` and
//! reads through the real `TcpFactory` and the Tokio `SystemClock`. The peer's script is
//! strict: it expects the exact request bytes and replies with the bytes of the Modbus
//! Application Protocol Specification V1.1b section 6.3 example (read registers 108 to 110,
//! answered 555, 0 and 100). Both the request and the reply are written out by hand from the
//! MBAP layout; no byte comes from the codec under test.
//!
//! Loopback tests cover bytes only (decision D21): nothing here waits on a deadline. The
//! only sockets are the peer's loopback listener and the driver's connection to it.
#![cfg(all(feature = "modbus", feature = "testkit"))]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, TcpListener};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use stonecrab::modbus::ModbusTcpDriver;
    use stonecrab::testkit::{Reaction, Script, ScriptedPeer, Step};
    use stonecrab::{
        ConnectionState, Deadline, DriverRegistry, ErrorKind, PlcValue, ReadRequest, ReadResponse,
        Tag, TagId,
    };
    use tokio::sync::Notify;

    /// Transaction 1, unit 1, FC03, wire address 107 (register 108), three registers.
    const REQUEST: [u8; 12] = [0, 1, 0, 0, 0, 6, 1, 3, 0, 0x6B, 0, 3];

    /// The matching reply: byte count 6, registers 0x022B, 0x0000 and 0x0064.
    const REPLY: [u8; 15] = [
        0, 1, 0, 0, 0, 9, 1, 3, 6, 0x02, 0x2B, 0x00, 0x00, 0x00, 0x64,
    ];

    /// The same read of register 108 alone: quantity 1.
    const ONE_REGISTER: [u8; 12] = [0, 1, 0, 0, 0, 6, 1, 3, 0, 0x6B, 0, 1];

    fn registry() -> DriverRegistry {
        let mut registry = DriverRegistry::new();
        registry
            .register(Arc::new(ModbusTcpDriver::new()))
            .expect("the scheme is free");
        registry
    }

    fn request(
        registry: &DriverRegistry,
        connection: &dyn stonecrab::Connection,
        addresses: &[(&str, &str)],
    ) -> ReadRequest {
        let driver = registry.driver("modbus-tcp").expect("registered");
        let mut builder = ReadRequest::builder(connection.limits())
            .deadline(Deadline::after(Instant::now(), Duration::from_secs(5)).expect("a deadline"));
        for (id, text) in addresses {
            let address = driver
                .parse_address(text, &stonecrab::AddressOptions::default())
                .expect("address");
            builder = builder.tag(Tag::new(TagId::new(id).expect("tag id"), address));
        }
        builder.build().expect("request")
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

    // covers: ALPHA_PLAN S26 / MB-TCP-CONNECT, MB-TCP-READ-HR over a loopback socket
    #[tokio::test(flavor = "current_thread")]
    async fn a_holding_register_read_over_loopback_returns_the_v1_1b_values() {
        let peer = ScriptedPeer::spawn(Script::strict(vec![
            Step::Expect(REQUEST.to_vec()),
            Step::Reply(REPLY.to_vec()),
        ]))
        .expect("the loopback peer");
        let registry = registry();
        let connection = registry
            .connect(&format!("modbus-tcp://127.0.0.1:{}", peer.port()))
            .await
            .expect("connect");
        assert_eq!(connection.state(), ConnectionState::Connected);

        let read = request(
            &registry,
            &*connection,
            &[
                ("r108", "holding-register:108:UINT"),
                ("r109", "holding-register:109:UINT"),
                ("r110", "holding-register:110:UINT"),
            ],
        );
        let response = connection.read(read).await.expect("read");
        assert_eq!(value(&response, "r108"), PlcValue::U16(555));
        assert_eq!(value(&response, "r109"), PlcValue::U16(0));
        assert_eq!(value(&response, "r110"), PlcValue::U16(100));
        assert_eq!(
            response.split_into().get(),
            1,
            "three adjacent registers are one wire request"
        );

        connection.close().await.expect("close");
        assert_eq!(connection.state(), ConnectionState::Closed);

        let report = peer.close().await.expect("the peer report");
        assert_eq!(report.received, REQUEST, "exactly the one request arrived");
        assert!(report.mismatches.is_empty(), "{:?}", report.mismatches);
        assert!(
            !report.failed && !report.closed,
            "the script ran to its end"
        );
        assert_eq!(report.ran, vec![0, 1], "expect then reply");
    }

    // covers: ALPHA_PLAN S26 / MB-TCP-CONNECT connect returns Connected over loopback
    #[tokio::test(flavor = "current_thread")]
    async fn connect_returns_connected_state() {
        let peer = ScriptedPeer::spawn(Script::strict(vec![])).expect("the loopback peer");
        let registry = registry();
        let connection = registry
            .connect(&format!("modbus-tcp:tcp://127.0.0.1:{}", peer.port()))
            .await
            .expect("connect");
        assert_eq!(connection.state(), ConnectionState::Connected);
        assert!(!connection.capabilities().write);
        connection.close().await.expect("close");
        let report = peer.close().await.expect("the peer report");
        assert!(report.received.is_empty(), "connecting sends nothing");
    }

    // covers: ALPHA_PLAN S26 / MB-TCP-CONNECT a refused connect is Disconnected
    #[tokio::test(flavor = "current_thread")]
    async fn connect_refused_yields_disconnected_error() {
        // Bind to learn a free port, then close the listener: the next connect is refused.
        let port = {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("a free port");
            listener.local_addr().expect("its address").port()
        };
        let error = registry()
            .connect(&format!("modbus-tcp://127.0.0.1:{port}"))
            .await
            .err()
            .expect("nothing listens there");
        assert_eq!(error.kind(), ErrorKind::Disconnected);
    }

    // covers: ALPHA_PLAN S26 / F3 device exception over a real socket
    #[tokio::test(flavor = "current_thread")]
    async fn an_exception_reply_over_loopback_is_a_per_item_device_error() {
        // Exception 02 (ILLEGAL DATA ADDRESS) for the same request: function 0x83.
        let exception = vec![0, 1, 0, 0, 0, 3, 1, 0x83, 2];
        let peer = ScriptedPeer::spawn(Script::strict(vec![
            Step::Expect(ONE_REGISTER.to_vec()),
            Step::Reply(exception),
        ]))
        .expect("the loopback peer");
        let registry = registry();
        let connection = registry
            .connect(&format!("modbus-tcp://127.0.0.1:{}", peer.port()))
            .await
            .expect("connect");
        let read = request(
            &registry,
            &*connection,
            &[("r108", "holding-register:108:UINT")],
        );
        let response = connection
            .read(read)
            .await
            .expect("a refused chunk is an item error");
        let error = response
            .get(&TagId::new("r108").expect("tag id"))
            .expect("the item")
            .outcome()
            .as_ref()
            .expect_err("refused");
        assert_eq!(error.kind(), ErrorKind::InvalidAddress);
        assert_eq!(error.status().map(stonecrab::ProtocolStatus::code), Some(2));
        connection.close().await.expect("close");
        let report = peer.close().await.expect("the peer report");
        assert!(report.mismatches.is_empty(), "{:?}", report.mismatches);
    }

    // covers: ALPHA_PLAN S26 / MB-TCP-CONNECT close fails the pending read with Disconnected
    #[tokio::test(flavor = "current_thread")]
    async fn close_fails_pending_read_with_disconnected() {
        // The peer swallows the request and says so; the close is issued only after the
        // request has reached it, so nothing here depends on timing.
        let arrived = Arc::new(Notify::new());
        let signal = Arc::clone(&arrived);
        let peer = ScriptedPeer::spawn(Script::new(vec![Step::react(move |_request| {
            signal.notify_one();
            Reaction::Silence
        })]))
        .expect("the loopback peer");
        let registry = registry();
        let connection = registry
            .connect(&format!("modbus-tcp://127.0.0.1:{}", peer.port()))
            .await
            .expect("connect");
        let read = request(
            &registry,
            &*connection,
            &[("r108", "holding-register:108:UINT")],
        );
        let (outcome, closed) = tokio::join!(connection.read(read), async {
            arrived.notified().await;
            connection.close().await
        });
        closed.expect("close");
        let error = outcome.expect_err("the pending read is failed by the close");
        assert_eq!(error.kind(), ErrorKind::Disconnected);
        drop(peer.close().await);
    }
}
