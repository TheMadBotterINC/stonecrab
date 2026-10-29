//! `TlsFactory` and the TLS transport over real loopback sockets (slice S71, ADR-0032).
//!
//! The server side is rustls's own `ServerConfig` behind tokio-rustls's acceptor, configured
//! here; the certificate authorities and every key are made at run time by
//! `support/tls_pki.rs` and written to a scratch directory, so nothing secret is committed.
//! Every listener binds `127.0.0.1:0`; nothing leaves the machine and nothing sleeps on real
//! time. Each call is bounded by a `Deadline` of a few seconds so a regression fails instead
//! of hanging; the deadline over a handshake that never completes is a unit test of the
//! transport, on the paused clock.
#![cfg(feature = "tls")]
#![forbid(unsafe_code)]

#[cfg(test)]
#[path = "support/tls_pki.rs"]
mod tls_pki;

#[cfg(test)]
mod tests {
    use std::io;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use bytes::{Bytes, BytesMut};
    use rustls::server::WebPkiClientVerifier;
    use rustls::{
        CertificateError, PeerIncompatible, ProtocolVersion, RootCertStore, ServerConfig,
        SupportedProtocolVersion,
    };
    use stonecrab::transport::{TlsFactory, TlsSettings, Transport, TransportFactory};
    use stonecrab::{Deadline, Endpoint, ErrorKind, Operation, PlcError, ResourceLimits, Result};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio_rustls::TlsAcceptor;
    use tokio_rustls::server::TlsStream;

    use crate::tls_pki::{Authority, Identity, Purpose, Scratch, SubjectName, Validity};

    const BOTH: &[&SupportedProtocolVersion] = &[&rustls::version::TLS13, &rustls::version::TLS12];
    const ONLY_12: &[&SupportedProtocolVersion] = &[&rustls::version::TLS12];
    const ONLY_13: &[&SupportedProtocolVersion] = &[&rustls::version::TLS13];

    /// The names of the loopback server: its IP address and `localhost`.
    const LOOPBACK: &[SubjectName] = &[
        SubjectName::Dns("localhost"),
        SubjectName::Ip([127, 0, 0, 1]),
    ];

    fn deadline() -> Deadline {
        Deadline::after(Instant::now(), Duration::from_secs(5)).expect("representable")
    }

    fn limits() -> ResourceLimits {
        ResourceLimits::builder()
            .max_frame_bytes(260)
            .max_buffered_bytes(520)
            .build()
            .expect("valid limits")
    }

    /// An authority, a server identity it issued and the trust store that names it.
    struct Pki {
        scratch: Scratch,
        authority: Authority,
        server: Identity,
        trust_store: PathBuf,
    }

    impl Pki {
        fn new() -> Self {
            let scratch = Scratch::new();
            let authority = Authority::new("stonecrab test authority");
            let server = authority.issue(LOOPBACK, Purpose::Server, Validity::Current);
            let trust_store = scratch.write("ca.pem", &authority.certificate_pem());
            Self {
                scratch,
                authority,
                server,
                trust_store,
            }
        }

        /// Settings trusting this authority, presenting `client` if given.
        fn settings(&self, client: Option<&Identity>) -> TlsSettings {
            let mut settings = TlsSettings::new(&self.trust_store);
            settings.keystore =
                client.map(|identity| self.scratch.write("client.pem", &identity.keystore_pem()));
            settings
        }

        fn factory(&self, client: Option<&Identity>) -> TlsFactory {
            TlsFactory::new(&self.settings(client)).expect("the PEM files load")
        }
    }

    /// A server configuration for `identity`, offering `versions`, requiring a client
    /// certificate issued by `clients` when that is given.
    fn server_config(
        identity: &Identity,
        versions: &[&'static SupportedProtocolVersion],
        clients: Option<&Authority>,
    ) -> Arc<ServerConfig> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let builder = ServerConfig::builder_with_provider(Arc::clone(&provider))
            .with_protocol_versions(versions)
            .expect("versions");
        let builder = match clients {
            None => builder.with_no_client_auth(),
            Some(authority) => {
                let mut roots = RootCertStore::empty();
                roots
                    .add(authority.certificate_der())
                    .expect("a trust anchor");
                let verifier =
                    WebPkiClientVerifier::builder_with_provider(Arc::new(roots), provider)
                        .build()
                        .expect("a client verifier");
                builder.with_client_cert_verifier(verifier)
            }
        };
        Arc::new(
            builder
                .with_single_cert(identity.chain(), identity.key())
                .expect("the server identity"),
        )
    }

    /// A listener on loopback and the endpoint naming it by `host`.
    async fn listen(host: &str) -> (TcpListener, Endpoint) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("address").port();
        let endpoint = Endpoint::Tcp {
            host: host.to_owned(),
            port,
        };
        (listener, endpoint)
    }

    /// Accept one connection and run the server's side of the handshake.
    async fn accept(
        listener: &TcpListener,
        config: Arc<ServerConfig>,
    ) -> io::Result<TlsStream<TcpStream>> {
        let (socket, _) = listener.accept().await?;
        TlsAcceptor::from(config).accept(socket).await
    }

    /// Open `factory` towards `endpoint` while the listener serves one handshake with
    /// `config`; both outcomes.
    async fn handshake(
        factory: &TlsFactory,
        listener: &TcpListener,
        endpoint: &Endpoint,
        config: Arc<ServerConfig>,
    ) -> (Result<Box<dyn Transport>>, io::Result<TlsStream<TcpStream>>) {
        let (opened, accepted) = tokio::join!(
            factory.open(endpoint, limits(), deadline()),
            accept(listener, config)
        );
        (opened, accepted)
    }

    fn refused(opened: Result<Box<dyn Transport>>) -> PlcError {
        opened.err().expect("the open must fail")
    }

    /// The rustls error behind a failed open, carried by the `io::Error` the handshake
    /// returned, so a test can tell which check refused.
    fn reason(error: &PlcError) -> &rustls::Error {
        std::error::Error::source(error)
            .and_then(|source| source.downcast_ref::<io::Error>())
            .and_then(io::Error::get_ref)
            .and_then(|inner| inner.downcast_ref::<rustls::Error>())
            .expect("a rustls error behind the io error")
    }

    // covers: S71 / UTR-TLS handshake, round trip and close over loopback
    #[tokio::test]
    async fn tls_handshake_with_a_trusted_server_succeeds() {
        let pki = Pki::new();
        let factory = pki.factory(None);
        for host in ["127.0.0.1", "localhost"] {
            let (listener, endpoint) = listen(host).await;
            let config = server_config(&pki.server, BOTH, None);
            let (opened, accepted) = handshake(&factory, &listener, &endpoint, config).await;
            let mut transport =
                opened.expect("a certificate for the host, from the trusted authority");
            let mut device = accepted.expect("the server's side of the handshake");
            assert_eq!(
                transport.peer(),
                &endpoint,
                "the endpoint it was opened for"
            );
            assert_eq!(transport.limits(), &limits(), "the limits it was given");
            assert!(!transport.delivers_datagrams(), "a byte stream");

            transport
                .send(Bytes::from_static(b"ping"), deadline())
                .await
                .expect("send");
            let mut request = [0_u8; 4];
            device
                .read_exact(&mut request)
                .await
                .expect("the device reads");
            assert_eq!(&request, b"ping", "plaintext arrives whole ({host})");

            device.write_all(b"pong").await.expect("the device answers");
            device.flush().await.expect("flush");
            let mut reply = BytesMut::new();
            while reply.len() < 4 {
                transport.recv(&mut reply, deadline()).await.expect("recv");
            }
            assert_eq!(&reply[..], b"pong", "the answer arrives decrypted ({host})");

            transport
                .shutdown(deadline())
                .await
                .expect("close_notify and FIN");
            let mut rest = Vec::new();
            let read = device.read_to_end(&mut rest).await.expect("a clean end");
            assert_eq!(read, 0, "close_notify ends the stream without bytes");
            transport
                .shutdown(Deadline::at(Instant::now()))
                .await
                .expect("a second shutdown is a no-op");
            let error = transport
                .send(Bytes::from_static(b"late"), deadline())
                .await
                .expect_err("closed");
            assert_eq!(error.kind(), ErrorKind::Disconnected, "after shutdown");
        }
    }

    // covers: S71 / UTR-TLS TLS 1.3 and 1.2 are offered, 1.3 is preferred
    #[tokio::test]
    async fn tls_defaults_offer_only_tls_1_3_and_1_2() {
        let pki = Pki::new();
        let factory = pki.factory(None);
        for (versions, want) in [
            (BOTH, ProtocolVersion::TLSv1_3),
            (ONLY_13, ProtocolVersion::TLSv1_3),
            (ONLY_12, ProtocolVersion::TLSv1_2),
        ] {
            let (listener, endpoint) = listen("127.0.0.1").await;
            let config = server_config(&pki.server, versions, None);
            let (opened, accepted) = handshake(&factory, &listener, &endpoint, config).await;
            opened.expect("the versions overlap");
            let device = accepted.expect("the server's side");
            assert_eq!(
                device.get_ref().1.protocol_version(),
                Some(want),
                "negotiated version"
            );
        }
    }

    // covers: SPEC mandatory-failure-cases / trust failure (untrusted certificate)
    #[tokio::test]
    async fn tls_untrusted_certificate_is_refused() {
        let pki = Pki::new();
        let stranger = Authority::new("another authority");
        let server = stranger.issue(LOOPBACK, Purpose::Server, Validity::Current);
        let (listener, endpoint) = listen("127.0.0.1").await;
        let config = server_config(&server, BOTH, None);
        let (opened, accepted) = handshake(&pki.factory(None), &listener, &endpoint, config).await;
        let error = refused(opened);
        assert_eq!(error.kind(), ErrorKind::Trust, "unknown issuer: {error}");
        assert!(
            matches!(
                reason(&error),
                rustls::Error::InvalidCertificate(CertificateError::UnknownIssuer)
            ),
            "the issuer check refused: {error}"
        );
        assert_eq!(
            error.diagnostics().operation,
            Some(Operation::Connect),
            "a connect failure"
        );
        assert!(accepted.is_err(), "the server saw the client's alert");
    }

    // covers: SPEC mandatory-failure-cases / trust failure (certificate for another name)
    #[tokio::test]
    async fn tls_host_name_mismatch_is_refused() {
        let pki = Pki::new();
        let factory = pki.factory(None);
        let elsewhere = pki.authority.issue(
            &[
                SubjectName::Dns("plc.example"),
                SubjectName::Ip([192, 0, 2, 10]),
            ],
            Purpose::Server,
            Validity::Current,
        );
        for host in ["127.0.0.1", "localhost"] {
            let (listener, endpoint) = listen(host).await;
            let config = server_config(&elsewhere, BOTH, None);
            let (opened, _accepted) = handshake(&factory, &listener, &endpoint, config).await;
            let error = refused(opened);
            assert_eq!(error.kind(), ErrorKind::Trust, "{host}: {error}");
            assert!(
                matches!(
                    reason(&error),
                    rustls::Error::InvalidCertificate(
                        CertificateError::NotValidForNameContext { .. }
                    )
                ),
                "the name check refused ({host}): {error}"
            );
        }
    }

    // covers: SPEC mandatory-failure-cases / trust failure (expired certificate)
    #[tokio::test]
    async fn an_expired_server_certificate_is_refused() {
        let pki = Pki::new();
        let expired = pki
            .authority
            .issue(LOOPBACK, Purpose::Server, Validity::Expired);
        let (listener, endpoint) = listen("127.0.0.1").await;
        let config = server_config(&expired, BOTH, None);
        let (opened, _accepted) = handshake(&pki.factory(None), &listener, &endpoint, config).await;
        let error = refused(opened);
        assert_eq!(error.kind(), ErrorKind::Trust, "expired: {error}");
        assert!(
            matches!(
                reason(&error),
                rustls::Error::InvalidCertificate(CertificateError::ExpiredContext { .. })
            ),
            "the validity check refused: {error}"
        );
    }

    // covers: S71 / UTR-TLS no plaintext fallback and no version below TLS 1.2
    #[tokio::test]
    async fn a_peer_answering_in_plaintext_or_below_tls_1_2_is_refused_after_the_client_hello() {
        let pki = Pki::new();
        let factory = pki.factory(None);
        // A Modbus/TCP read-holding-registers response, as a plaintext device would send it.
        let plaintext = vec![
            0x00, 0x01, 0x00, 0x00, 0x00, 0x05, 0x01, 0x03, 0x02, 0x00, 0x2A,
        ];
        // A TLS 1.0 ServerHello choosing TLS_RSA_WITH_AES_128_CBC_SHA (RFC 2246, 7.4.1.3).
        let mut tls_1_0 = vec![
            0x16, 0x03, 0x01, 0x00, 0x2A, 0x02, 0x00, 0x00, 0x26, 0x03, 0x01,
        ];
        tls_1_0.extend([0x5A; 32]);
        tls_1_0.extend([0x00, 0x00, 0x2F, 0x00]);
        for (answer, name) in [(plaintext, "plaintext"), (tls_1_0, "TLS 1.0")] {
            let (listener, endpoint) = listen("127.0.0.1").await;
            let device = async {
                let (mut socket, _) = listener.accept().await.expect("accept");
                let mut header = [0_u8; 5];
                socket
                    .read_exact(&mut header)
                    .await
                    .expect("a record header");
                let [content, _, _, high, low] = header;
                let mut body = vec![0_u8; usize::from(u16::from_be_bytes([high, low]))];
                socket.read_exact(&mut body).await.expect("the record");
                socket.write_all(&answer).await.expect("the answer");
                let mut rest = Vec::new();
                if let Err(error) = socket.read_to_end(&mut rest).await {
                    assert_eq!(error.kind(), io::ErrorKind::ConnectionReset, "{error}");
                }
                (content, body.first().copied(), rest)
            };
            let (opened, (content, handshake, rest)) =
                tokio::join!(factory.open(&endpoint, limits(), deadline()), device);
            let error = refused(opened);
            assert_eq!(error.kind(), ErrorKind::Disconnected, "{name}: {error}");
            let refused_by = reason(&error);
            assert!(
                matches!(
                    (name, refused_by),
                    ("plaintext", rustls::Error::InvalidMessage(_))
                        | (
                            "TLS 1.0",
                            rustls::Error::PeerIncompatible(
                                PeerIncompatible::ServerDoesNotSupportTls12Or13
                            )
                        )
                ),
                "{name}: {refused_by:?}"
            );
            assert_eq!(content, 0x16, "{name}: the first record is a handshake");
            assert_eq!(handshake, Some(0x01), "{name}: and it is a ClientHello");
            assert!(
                rest.first().is_none_or(|&content| content == 0x15),
                "{name}: at most an alert follows, never a request: {rest:?}"
            );
        }
    }

    // covers: S71 / UTR-TLS-TRUST mutual TLS with a client certificate from the keystore
    #[tokio::test]
    async fn tls_client_certificate_is_presented_for_mutual_tls() {
        let pki = Pki::new();
        let client = pki.authority.issue(
            &[SubjectName::Dns("client.example")],
            Purpose::Client,
            Validity::Current,
        );
        let factory = pki.factory(Some(&client));
        for versions in [ONLY_13, ONLY_12] {
            let (listener, endpoint) = listen("127.0.0.1").await;
            let config = server_config(&pki.server, versions, Some(&pki.authority));
            let (opened, accepted) = handshake(&factory, &listener, &endpoint, config).await;
            let mut transport = opened.expect("the server accepts the client certificate");
            let mut device = accepted.expect("the server's side");
            let presented = device
                .get_ref()
                .1
                .peer_certificates()
                .and_then(|chain| chain.first().cloned());
            assert_eq!(
                presented,
                client.chain().first().cloned(),
                "the keystore's certificate"
            );
            transport
                .send(Bytes::from_static(b"mtls"), deadline())
                .await
                .expect("send");
            let mut request = [0_u8; 4];
            device.read_exact(&mut request).await.expect("read");
            assert_eq!(&request, b"mtls", "the exchange works");
        }
    }

    // covers: SPEC mandatory-failure-cases / authentication failure (client certificate refused)
    #[tokio::test]
    async fn a_server_requiring_a_client_certificate_refuses_a_client_without_a_good_one() {
        let pki = Pki::new();
        let stranger = Authority::new("another authority");
        let foreign = stranger.issue(
            &[SubjectName::Dns("client.example")],
            Purpose::Client,
            Validity::Current,
        );
        for client in [None, Some(&foreign)] {
            let factory = pki.factory(client);
            // TLS 1.2: the server judges the client certificate inside the handshake.
            let (listener, endpoint) = listen("127.0.0.1").await;
            let config = server_config(&pki.server, ONLY_12, Some(&pki.authority));
            let (opened, accepted) = handshake(&factory, &listener, &endpoint, config).await;
            let error = refused(opened);
            assert_eq!(error.kind(), ErrorKind::Authentication, "TLS 1.2: {error}");
            assert!(accepted.is_err(), "the server refused");

            // TLS 1.3: the client has finished when the server judges it, so the refusal
            // arrives with the first receive.
            let (listener, endpoint) = listen("127.0.0.1").await;
            let config = server_config(&pki.server, ONLY_13, Some(&pki.authority));
            let (opened, accepted) = handshake(&factory, &listener, &endpoint, config).await;
            let mut transport = opened.expect("the client's side completes first under TLS 1.3");
            assert!(accepted.is_err(), "the server refused");
            let mut buffer = BytesMut::new();
            let error = transport
                .recv(&mut buffer, deadline())
                .await
                .expect_err("the server's alert");
            assert_eq!(error.kind(), ErrorKind::Authentication, "TLS 1.3: {error}");
            let error = transport
                .recv(&mut buffer, deadline())
                .await
                .expect_err("unusable");
            assert_eq!(
                error.kind(),
                ErrorKind::Disconnected,
                "the transport is spent"
            );
        }
    }

    // covers: SPEC mandatory-failure-cases / F2 transport (peer close, frame bound, elapsed deadline)
    #[tokio::test]
    async fn bounds_deadlines_and_a_closing_peer_follow_the_tcp_rules() {
        let pki = Pki::new();
        let factory = pki.factory(None);
        let (listener, endpoint) = listen("127.0.0.1").await;
        let config = server_config(&pki.server, BOTH, None);
        let (opened, accepted) = handshake(&factory, &listener, &endpoint, config).await;
        let mut transport = opened.expect("open");
        let mut device = accepted.expect("accept");

        let error = transport
            .send(Bytes::from(vec![0_u8; 261]), deadline())
            .await
            .expect_err("over the frame bound");
        assert_eq!(error.kind(), ErrorKind::ResourceLimit, "frame bound");
        let error = transport
            .send(Bytes::from_static(b"x"), Deadline::at(Instant::now()))
            .await
            .expect_err("elapsed");
        assert_eq!(
            error.kind(),
            ErrorKind::Timeout,
            "elapsed deadline before any I/O"
        );
        let mut full = BytesMut::from(&[0_u8; 520][..]);
        let error = transport
            .recv(&mut full, deadline())
            .await
            .expect_err("full buffer");
        assert_eq!(error.kind(), ErrorKind::ResourceLimit, "buffer bound");
        transport
            .send(Bytes::from_static(b"still usable"), deadline())
            .await
            .expect("refusals before I/O leave the transport usable");

        device.write_all(b"bye").await.expect("write");
        device.shutdown().await.expect("close_notify");
        let mut buffer = BytesMut::new();
        let mut reads = Vec::new();
        loop {
            let read = transport.recv(&mut buffer, deadline()).await.expect("recv");
            reads.push(read);
            if read == 0 {
                break;
            }
        }
        assert_eq!(&buffer[..], b"bye", "the bytes before the close");
        let error = transport
            .recv(&mut buffer, deadline())
            .await
            .expect_err("closed");
        assert_eq!(
            error.kind(),
            ErrorKind::Disconnected,
            "after the peer's close_notify"
        );
    }

    // covers: S71 / UTR-TLS a peer that drops TCP without close_notify
    #[tokio::test]
    async fn a_peer_that_drops_the_connection_without_close_notify_is_disconnected() {
        let pki = Pki::new();
        let (listener, endpoint) = listen("127.0.0.1").await;
        let config = server_config(&pki.server, BOTH, None);
        let (opened, accepted) = handshake(&pki.factory(None), &listener, &endpoint, config).await;
        let mut transport = opened.expect("open");
        let (socket, _session) = accepted.expect("accept").into_inner();
        drop(socket);
        let error = transport
            .recv(&mut BytesMut::new(), deadline())
            .await
            .expect_err("truncated stream");
        assert_eq!(
            error.kind(),
            ErrorKind::Disconnected,
            "no close_notify: {error}"
        );
    }

    // covers: SPEC mandatory-failure-cases / invalid configuration (trust store and keystore files)
    #[test]
    fn unreadable_or_unusable_pem_files_are_invalid_config_without_paths() {
        let pki = Pki::new();
        let client = pki.authority.issue(
            &[SubjectName::Dns("client.example")],
            Purpose::Client,
            Validity::Current,
        );
        let other = pki.authority.issue(
            &[SubjectName::Dns("other.example")],
            Purpose::Client,
            Validity::Current,
        );
        let empty = pki.scratch.write("empty.pem", "no PEM here\n");
        let large = pki.scratch.write("large.pem", &"#".repeat(1_048_577));
        let broken = pki.scratch.write(
            "broken.pem",
            "-----BEGIN CERTIFICATE-----\n!!!!\n-----END CERTIFICATE-----\n",
        );
        let not_a_certificate = pki.scratch.write(
            "garbage.pem",
            "-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n",
        );
        let key_only = pki.scratch.write("key.pem", &client.key_pem());
        let certificate_only = pki
            .scratch
            .write("certificate.pem", &client.certificate_pem());
        let mismatched = pki.scratch.write(
            "mismatched.pem",
            &format!("{}{}", client.certificate_pem(), other.key_pem()),
        );
        let missing = pki.scratch.path().join("missing.pem");
        let scratch = pki.scratch.path().to_string_lossy().into_owned();
        let key_text: Vec<String> = [client.key_pem(), other.key_pem()]
            .iter()
            .flat_map(|pem| {
                pem.lines()
                    .filter(|line| !line.starts_with("-----") && line.len() >= 16)
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .collect();
        assert!(!key_text.is_empty(), "the keys have base64 text");

        let trust = |path: &PathBuf, want: &str| {
            let error = TlsFactory::new(&TlsSettings::new(path)).expect_err("refused");
            (error, want.to_owned())
        };
        let keystore = |path: &PathBuf, want: &str| {
            let mut settings = TlsSettings::new(&pki.trust_store);
            settings.keystore = Some(path.clone());
            let error = TlsFactory::new(&settings).expect_err("refused");
            (error, want.to_owned())
        };
        for (error, want) in [
            trust(&missing, "tls.trust-store cannot be opened"),
            trust(&empty, "tls.trust-store holds no PEM certificate"),
            trust(&large, "tls.trust-store is larger than 1 MiB"),
            trust(
                &broken,
                "tls.trust-store holds a PEM section that cannot be decoded",
            ),
            trust(
                &not_a_certificate,
                "tls.trust-store holds a certificate that is not a usable trust anchor",
            ),
            keystore(&missing, "tls.keystore cannot be opened"),
            keystore(&key_only, "tls.keystore holds no PEM certificate"),
            keystore(
                &certificate_only,
                "tls.keystore holds no unencrypted PEM private key",
            ),
            keystore(
                &broken,
                "tls.keystore holds a PEM section that cannot be decoded",
            ),
            keystore(
                &mismatched,
                "tls.keystore holds a key that does not match its certificate",
            ),
        ] {
            assert_eq!(error.kind(), ErrorKind::InvalidConfig, "{want}");
            let text = error.to_string();
            assert!(text.starts_with(&want), "{text} starts with {want}");
            // Neither the error nor any source in its chain, displayed or debugged, shows a
            // path or the text of a key.
            let mut shown = vec![text, format!("{error:?}")];
            let mut next = std::error::Error::source(&error);
            while let Some(source) = next {
                shown.push(source.to_string());
                shown.push(format!("{source:?}"));
                next = source.source();
            }
            for text in shown {
                assert!(!text.contains(&scratch), "no path in {text}");
                assert!(
                    key_text.iter().all(|line| !text.contains(line.as_str())),
                    "no key text in {text}"
                );
            }
        }
    }

    // covers: SPEC mandatory-failure-cases / invalid configuration (endpoint and server name)
    #[tokio::test]
    async fn an_endpoint_that_cannot_carry_a_server_name_is_invalid_config() {
        let pki = Pki::new();
        let factory = pki.factory(None);
        for endpoint in [
            Endpoint::Custom("127.0.0.1:802".to_owned()),
            Endpoint::Tcp {
                host: "not a name".to_owned(),
                port: 802,
            },
        ] {
            let error = refused(factory.open(&endpoint, limits(), deadline()).await);
            assert_eq!(error.kind(), ErrorKind::InvalidConfig, "{endpoint}");
            assert_eq!(
                error.diagnostics().operation,
                Some(Operation::Connect),
                "{endpoint}"
            );
        }
        let (listener, endpoint) = listen("127.0.0.1").await;
        drop(listener);
        let error = refused(factory.open(&endpoint, limits(), deadline()).await);
        assert_eq!(error.kind(), ErrorKind::Disconnected, "a closed port");
    }

    // covers: S71 / UTR-TLS-TRUST the keystore path and the key never reach Debug output
    #[test]
    fn tls_keystore_path_is_redacted_in_debug_output() {
        let pki = Pki::new();
        let client = pki.authority.issue(
            &[SubjectName::Dns("client.example")],
            Purpose::Client,
            Validity::Current,
        );
        let settings = pki.settings(Some(&client));
        let keystore = settings.keystore.clone().expect("a keystore");
        let shown = format!("{settings:?}");
        assert!(
            shown.contains("ca.pem"),
            "the trust store is shown: {shown}"
        );
        assert!(
            shown.contains("[REDACTED]"),
            "the keystore is marked: {shown}"
        );
        assert!(
            !shown.contains(keystore.to_string_lossy().as_ref()) && !shown.contains("client.pem"),
            "the keystore path is not shown: {shown}"
        );
        let factory = TlsFactory::new(&settings).expect("load");
        assert_eq!(
            format!("{factory:?}"),
            "TlsFactory { .. }",
            "nothing of the configuration"
        );
    }
}
