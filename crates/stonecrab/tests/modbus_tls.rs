//! Modbus TCP over TLS (`modbus-tcp:tls://`, slice S71, ADR-0032) against a first-party
//! device behind rustls on a loopback socket: reads, a write read back and ping over mutual
//! TLS, a server certificate from another authority, a server that refuses the client
//! certificate, and the redacted display of the configuration.
//!
//! The device is the MBAP responder of the contract runs (`tests/support/responder.rs`) and
//! the interop register map, behind a TLS acceptor that reads one MBAP request at a time and
//! writes the reply. The certificate authorities and every key are made at run time by
//! `tests/support/tls_pki.rs`, so no private key is committed. The driver uses its real TLS
//! transport and the system clock; every socket is on `127.0.0.1`, and nothing waits on real
//! time beyond a bounded socket wait.
#![cfg(all(feature = "modbus", feature = "testkit", feature = "tls"))]
#![forbid(unsafe_code)]

#[cfg(test)]
#[path = "support/responder.rs"]
#[expect(
    dead_code,
    reason = "the module is shared with the contract runs; these tests arm no fault"
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
#[path = "support/tls_pki.rs"]
#[expect(
    dead_code,
    reason = "the module is shared with the transport tests; these tests write no lone key"
)]
mod tls_pki;

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::path::Path;
    use std::sync::Arc;
    use std::time::Duration;

    use rustls::server::WebPkiClientVerifier;
    use rustls::{RootCertStore, ServerConfig, SupportedProtocolVersion};
    use stonecrab::modbus::ModbusTcpDriver;
    use stonecrab::testkit::{InteropMap, Reaction, Reactor};
    use stonecrab::{Driver, ErrorKind, PlcValue};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::task::JoinSet;
    use tokio_rustls::TlsAcceptor;

    use crate::interop::{Peer, value_of};
    use crate::responder::Device;
    use crate::tls_pki::{Authority, Identity, Purpose, Scratch, SubjectName, Validity};

    /// A holding register of the interop map: logical 13 is the UINT 42424.
    const READABLE: &str = "holding-register:13:UINT";

    /// A holding register of the write area 100 to 200, which starts at zero.
    const WRITABLE: &str = "holding-register:100:UINT";

    /// The length of an MBAP header.
    const MBAP_HEADER: usize = 7;

    fn register_map() -> InteropMap {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/modbus/interop/register-map.toml");
        InteropMap::load(&path).expect("the interop register map loads")
    }

    /// The device behind a TLS acceptor on loopback; its task is aborted when this is dropped.
    struct TlsDevice {
        address: SocketAddr,
        _task: JoinSet<()>,
    }

    impl TlsDevice {
        /// Serve `device` over TLS with `identity`, offering `versions`, requiring a client
        /// certificate from `clients` when that is given. Each connection is served until the
        /// client closes it.
        async fn spawn(
            device: &Device,
            identity: &Identity,
            versions: &[&'static SupportedProtocolVersion],
            clients: Option<&Authority>,
        ) -> Self {
            let provider = Arc::new(rustls::crypto::ring::default_provider());
            let builder = ServerConfig::builder_with_provider(Arc::clone(&provider))
                .with_protocol_versions(versions)
                .expect("versions");
            let builder = match clients {
                None => builder.with_no_client_auth(),
                Some(authority) => {
                    let mut roots = RootCertStore::empty();
                    roots.add(authority.certificate_der()).expect("anchor");
                    builder.with_client_cert_verifier(
                        WebPkiClientVerifier::builder_with_provider(Arc::new(roots), provider)
                            .build()
                            .expect("verifier"),
                    )
                }
            };
            let config = builder
                .with_single_cert(identity.chain(), identity.key())
                .expect("identity");
            let acceptor = TlsAcceptor::from(Arc::new(config));
            let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
            let address = listener.local_addr().expect("address");
            let mut react = device.reactor();
            let mut task = JoinSet::new();
            task.spawn(async move {
                while let Ok((socket, _)) = listener.accept().await {
                    let Ok(mut stream) = acceptor.accept(socket).await else {
                        continue;
                    };
                    serve(&mut stream, &mut react).await;
                }
            });
            Self {
                address,
                _task: task,
            }
        }

        fn uri(&self, query: &str) -> String {
            format!("modbus-tcp:tls://{}?{query}", self.address)
        }
    }

    /// Answer MBAP requests on one stream until it ends or the device closes it.
    async fn serve(
        stream: &mut tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
        react: &mut Reactor,
    ) {
        loop {
            let mut header = [0_u8; MBAP_HEADER];
            if stream.read_exact(&mut header).await.is_err() {
                return;
            }
            let [_, _, _, _, high, low, _] = header;
            let rest = usize::from(u16::from_be_bytes([high, low])).saturating_sub(1);
            let mut body = vec![0_u8; rest];
            if stream.read_exact(&mut body).await.is_err() {
                return;
            }
            let request = [&header[..], &body[..]].concat();
            let (reply, close) = match react(&request) {
                Reaction::Reply(reply) => (reply, false),
                Reaction::ReplyThenClose(reply) => (reply, true),
                Reaction::Silence => continue,
                Reaction::Close => (Vec::new(), true),
            };
            if stream.write_all(&reply).await.is_err() || stream.flush().await.is_err() || close {
                return;
            }
        }
    }

    /// An authority, the loopback server it issued, a client it issued, and the trust store
    /// and keystore files.
    struct Pki {
        scratch: Scratch,
        authority: Authority,
        server: Identity,
        trust_store: String,
        keystore: String,
    }

    impl Pki {
        fn new() -> Self {
            let scratch = Scratch::new();
            let authority = Authority::new("stonecrab modbus test authority");
            let server = authority.issue(
                &[SubjectName::Ip([127, 0, 0, 1])],
                Purpose::Server,
                Validity::Current,
            );
            let client = authority.issue(
                &[SubjectName::Dns("client.example")],
                Purpose::Client,
                Validity::Current,
            );
            let trust_store = path(&scratch.write("ca.pem", &authority.certificate_pem()));
            let keystore = path(&scratch.write("client.pem", &client.keystore_pem()));
            Self {
                scratch,
                authority,
                server,
                trust_store,
                keystore,
            }
        }

        /// The query naming the trust store and, when `mutual`, the keystore.
        fn query(&self, mutual: bool) -> String {
            let trust = format!("tls.trust-store={}", self.trust_store);
            if mutual {
                format!("{trust}&tls.keystore={}", self.keystore)
            } else {
                trust
            }
        }
    }

    fn path(path: &Path) -> String {
        path.to_str().expect("a UTF-8 temporary path").to_owned()
    }

    const BOTH: &[&SupportedProtocolVersion] = &[&rustls::version::TLS13, &rustls::version::TLS12];

    // covers: S71 / MB-TCP-TRANSPORTS-TLS reads, a write read back and ping over mutual TLS
    #[tokio::test]
    async fn reads_writes_and_ping_travel_over_mutual_tls() {
        let pki = Pki::new();
        let device = Device::from_map(&register_map());
        let server = TlsDevice::spawn(&device, &pki.server, BOTH, Some(&pki.authority)).await;
        let uri = server.uri(&format!("{}&access=read-write", pki.query(true)));
        let peer = Peer::connect(&uri, "loopback TLS").await;
        let response = peer.read(vec![peer.tag("x", READABLE)]).await;
        assert_eq!(value_of(&response, "x"), Some(PlcValue::U16(42_424)));
        let written = peer
            .write(vec![(peer.tag("w", WRITABLE), PlcValue::U16(4_321))])
            .await;
        assert_eq!(written.confirmed_count(), 1, "{written:?}");
        let response = peer.read(vec![peer.tag("w", WRITABLE)]).await;
        assert_eq!(value_of(&response, "w"), Some(PlcValue::U16(4_321)));
        peer.connection.ping().await.expect("ping over TLS");
        peer.close().await;
        assert_eq!(device.applied(), 1, "the write reached the device once");
    }

    // covers: SPEC mandatory-failure-cases / trust failure (Modbus over TLS, unknown authority)
    #[tokio::test]
    async fn a_server_certificate_from_another_authority_fails_connect_with_trust() {
        let pki = Pki::new();
        let stranger = Authority::new("another authority");
        let impostor = stranger.issue(
            &[SubjectName::Ip([127, 0, 0, 1])],
            Purpose::Server,
            Validity::Current,
        );
        let device = Device::from_map(&register_map());
        let server = TlsDevice::spawn(&device, &impostor, BOTH, None).await;
        let driver = ModbusTcpDriver::new();
        let config = driver
            .parse_config(&server.uri(&pki.query(false)))
            .expect("config");
        let error = driver
            .connect(config)
            .await
            .err()
            .expect("the certificate is not trusted");
        assert_eq!(error.kind(), ErrorKind::Trust, "{error}");
        assert_eq!(device.reads(), 0, "no request was sent");
    }

    // covers: SPEC mandatory-failure-cases / authentication failure (Modbus over TLS 1.3)
    #[tokio::test]
    async fn a_tls_1_3_server_requiring_a_client_certificate_fails_the_first_request() {
        let pki = Pki::new();
        let device = Device::from_map(&register_map());
        let server = TlsDevice::spawn(
            &device,
            &pki.server,
            &[&rustls::version::TLS13],
            Some(&pki.authority),
        )
        .await;
        let peer = Peer::connect(&server.uri(&pki.query(false)), "loopback TLS 1.3").await;
        let error = peer
            .try_read(vec![peer.tag("x", READABLE)], Duration::from_secs(2))
            .await
            .expect_err("the server refused the missing certificate");
        assert_eq!(error.kind(), ErrorKind::Authentication, "{error}");
        assert_eq!(device.reads(), 0, "the device never saw the request");
    }

    // covers: S71 / ADR-0032 the redacted display shows the trust store but not the keystore path
    #[test]
    fn the_redacted_display_hides_the_keystore_path() {
        let pki = Pki::new();
        let config = ModbusTcpDriver::new()
            .parse_config(&format!("modbus-tcp:tls://127.0.0.1?{}", pki.query(true)))
            .expect("config");
        let shown = config.redacted_display();
        assert!(shown.contains(&pki.trust_store), "the trust store: {shown}");
        assert!(
            shown.contains("[REDACTED]"),
            "the keystore is marked: {shown}"
        );
        assert!(
            !shown.contains(&pki.keystore),
            "not the keystore path: {shown}"
        );
        assert!(!format!("{config:?}").contains(&pki.keystore), "nor Debug");
        assert!(
            pki.scratch.path().exists(),
            "the files exist while the test runs"
        );
    }
}
