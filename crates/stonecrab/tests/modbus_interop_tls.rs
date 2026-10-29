//! Interoperability over TLS (slice S71, ADR-0032): the driver's `modbus-tcp:tls://`
//! transport against pymodbus 3.8.6 in its TLS mode (MBAP inside TLS 1.2 or 1.3 on Python's
//! ssl module, which is OpenSSL), in the container of `interop/pymodbus/`, with mutual TLS and
//! a PKI that openssl makes at run time.
//!
//! Every test here is `#[ignore]`d and runs only when `STONECRAB_INTEROP_MODBUS_TLS` names the
//! server up to the `?` (for example `modbus-tcp:tls://127.0.0.1:8802`) and
//! `STONECRAB_INTEROP_TLS_PKI` names the directory with the client's side of the PKI
//! (`ca.pem`, `other-ca.pem`, `keystore.pem`): `mise run interop-tls` (and `mise run
//! interop`) makes the PKI, starts the container and runs them, and so does the
//! `interop-tls` job of `.github/workflows/ci.yml`. Without the variables the tests stay
//! ignored and nothing here touches the network.
//!
//! The server requires a client certificate issued by its authority; the certificates are
//! ECDSA P-256 from openssl, an implementation independent of the test PKI of
//! `tests/support/tls_pki.rs`. The expected values are those of
//! `fixtures/modbus/interop/register-map.toml`, as in the plain pymodbus run.
//!
//! Time: every read and write has a 2 s deadline taken from the clock the driver uses and no
//! test sleeps.
#![cfg(all(feature = "modbus", feature = "testkit", feature = "tls"))]
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
    use std::time::Duration;

    use stonecrab::modbus::ModbusTcpDriver;
    use stonecrab::{Driver, ErrorKind, PlcValue};

    use crate::interop::{Peer, value_of};

    /// The environment variable that names the server, up to the `?`.
    const VARIABLE: &str = "STONECRAB_INTEROP_MODBUS_TLS";

    /// The environment variable that names the client's side of the PKI.
    const PKI: &str = "STONECRAB_INTEROP_TLS_PKI";

    /// A holding register of the interop map: logical 13 is the UINT 42424.
    const READABLE: &str = "holding-register:13:UINT";

    /// A register of the write area 100 to 200 that the plain pymodbus tests do not use.
    const WRITABLE: &str = "holding-register:150:UINT";

    /// The connection string for the server with `query` after the `?`.
    fn uri(query: &str) -> String {
        let server = std::env::var(VARIABLE).unwrap_or_else(|_| {
            panic!(
                "{VARIABLE} is not set: these tests need the pymodbus TLS server of \
                 interop/pymodbus (run `mise run interop-tls`)"
            )
        });
        format!("{server}?{query}")
    }

    /// A file of the client's side of the PKI.
    fn pki(file: &str) -> String {
        let dir = std::env::var(PKI).unwrap_or_else(|_| {
            panic!("{PKI} is not set: run `mise run interop-tls`, which makes the PKI")
        });
        format!("{dir}/{file}")
    }

    // covers: S71 / MB-TCP-TRANSPORTS-TLS reads, a write read back and ping against an independent TLS server
    #[tokio::test]
    #[ignore = "interop: set STONECRAB_INTEROP_MODBUS_TLS and STONECRAB_INTEROP_TLS_PKI"]
    async fn interop_tls_reads_writes_and_pings_over_mutual_tls() {
        let query = format!(
            "tls.trust-store={}&tls.keystore={}&access=read-write",
            pki("ca.pem"),
            pki("keystore.pem")
        );
        let peer = Peer::connect(&uri(&query), "Start it with `mise run interop-tls`").await;
        let response = peer.read(vec![peer.tag("x", READABLE)]).await;
        assert_eq!(value_of(&response, "x"), Some(PlcValue::U16(42_424)));
        for value in [7_007, 0] {
            let written = peer
                .write(vec![(peer.tag("w", WRITABLE), PlcValue::U16(value))])
                .await;
            assert_eq!(written.confirmed_count(), 1, "{written:?}");
            let response = peer.read(vec![peer.tag("w", WRITABLE)]).await;
            assert_eq!(value_of(&response, "w"), Some(PlcValue::U16(value)));
        }
        peer.connection.ping().await.expect("ping over TLS");
        peer.close().await;
    }

    // covers: SPEC mandatory-failure-cases / trust failure against an independent TLS server
    #[tokio::test]
    #[ignore = "interop: set STONECRAB_INTEROP_MODBUS_TLS and STONECRAB_INTEROP_TLS_PKI"]
    async fn interop_tls_a_server_certificate_from_another_authority_is_a_trust_error() {
        let query = format!(
            "tls.trust-store={}&tls.keystore={}",
            pki("other-ca.pem"),
            pki("keystore.pem")
        );
        let driver = ModbusTcpDriver::new();
        let config = driver.parse_config(&uri(&query)).expect("config");
        let error = driver
            .connect(config)
            .await
            .err()
            .expect("the server's authority is not trusted");
        assert_eq!(error.kind(), ErrorKind::Trust, "{error}");
    }

    // covers: SPEC mandatory-failure-cases / F2 transport (a TLS server that drops a client without a certificate)
    #[tokio::test]
    #[ignore = "interop: set STONECRAB_INTEROP_MODBUS_TLS and STONECRAB_INTEROP_TLS_PKI"]
    async fn interop_tls_without_a_client_certificate_the_server_drops_the_connection() {
        // Under TLS 1.3 the client has finished its handshake before the server judges the
        // client certificate, so the connection opens. pymodbus's asyncio TLS server then
        // closes the TCP connection without sending its alert (openssl s_client sees the same:
        // a reset under TLS 1.3, an unexpected end of stream under TLS 1.2), so the driver
        // cannot tell the refusal from a lost link: the request fails with `Disconnected`. A
        // server that sends the alert gives `Authentication` (`tests/modbus_tls.rs`).
        let query = format!("tls.trust-store={}", pki("ca.pem"));
        let peer = Peer::connect(&uri(&query), "Start it with `mise run interop-tls`").await;
        let error = peer
            .try_read(vec![peer.tag("x", READABLE)], Duration::from_secs(2))
            .await
            .expect_err("the server requires a client certificate");
        assert_eq!(error.kind(), ErrorKind::Disconnected, "{error}");
    }
}
