use std::fmt;
use std::fs::File;
use std::future::Future;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::{Deadline, Endpoint, ErrorKind, OpFuture, PlcError, ResourceLimits, Result};
use bytes::{BufMut, Bytes, BytesMut};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use rustls::{AlertDescription, ClientConfig, RootCertStore};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;

use crate::transport::deadline::{Site, ensure_before, within};
use crate::transport::tcp::resolve_and_connect;
use crate::transport::{Transport, TransportFactory};

/// The largest PEM file [`TlsFactory::new`] reads. A trust store of a few hundred
/// certificates fits many times over; anything larger is refused before it is parsed.
const MAX_PEM_BYTES: u64 = 1 << 20;

/// The connection-string name of the trust store, used in the refusals.
const TRUST_STORE: &str = "tls.trust-store";

/// The connection-string name of the client keystore, used in the refusals.
const KEYSTORE: &str = "tls.keystore";

/// Where a [`TlsFactory`] finds its trust anchors and, for mutual TLS, its own certificate
/// and key: two PEM files.
///
/// `trust_store` holds the certificates of the authorities a server certificate must chain
/// to; nothing else is trusted (no web PKI roots, no platform certificate store). `keystore`,
/// when set, holds the client certificate chain (leaf first) followed by its private key
/// (PKCS #8, SEC1 or PKCS #1, unencrypted), the certificate and key a server that asks for
/// one gets. The names are those of upstream's `tls.trust-store` and `tls.keystore`
/// options, read as PEM where upstream reads PKCS #12 or JKS.
///
/// `Debug` shows the trust-store path and only whether a keystore is set, never its path:
/// the trust store holds public certificates, the keystore a private key, and a log line
/// does not say where that lives (ADR-0032).
///
/// # Compatibility: docs/parity/transport.md#UTR-TLS-TRUST
/// Upstream's `trust-store` and `keystore` are PKCS #12 or JKS files with passwords; these
/// are PEM files without one, and the other options of that row are not offered.
///
/// # Examples
/// ```
/// use stonecrab::transport::TlsSettings;
///
/// let mut settings = TlsSettings::new("/etc/plc/ca.pem");
/// settings.keystore = Some("/etc/plc/client.pem".into());
/// let shown = format!("{settings:?}");
/// assert!(shown.contains("/etc/plc/ca.pem"));
/// assert!(!shown.contains("client.pem"));
/// ```
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct TlsSettings {
    /// The PEM file of the trusted certificate authorities (`tls.trust-store`).
    pub trust_store: PathBuf,
    /// The PEM file of the client certificate chain and its private key, for mutual TLS
    /// (`tls.keystore`); `None` presents no client certificate.
    pub keystore: Option<PathBuf>,
}

impl TlsSettings {
    /// Settings that trust the authorities in `trust_store` and present no client
    /// certificate; assign [`keystore`](Self::keystore) for mutual TLS.
    ///
    /// Nothing is read until [`TlsFactory::new`].
    ///
    /// # Examples
    /// ```
    /// use stonecrab::transport::TlsSettings;
    ///
    /// let settings = TlsSettings::new("ca.pem");
    /// assert_eq!(settings.trust_store.to_str(), Some("ca.pem"));
    /// assert!(settings.keystore.is_none());
    /// ```
    #[must_use]
    pub fn new(trust_store: impl Into<PathBuf>) -> Self {
        Self {
            trust_store: trust_store.into(),
            keystore: None,
        }
    }
}

/// The keystore path is never shown (ADR-0032).
impl fmt::Debug for TlsSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TlsSettings")
            .field("trust_store", &self.trust_store)
            .field(
                "keystore",
                &self.keystore.as_ref().map(|_path| "[REDACTED]"),
            )
            .finish()
    }
}

/// Opens TLS transports: a TCP connection, then a TLS 1.3 or 1.2 handshake that verifies the
/// server against the caller's trust anchors.
///
/// [`new`](Self::new) reads the [`TlsSettings`] files once and builds one rustls client
/// configuration on the ring provider, which every [`open`](TransportFactory::open) of the
/// factory shares (so a reconnect may resume the TLS session). `open` takes an
/// `Endpoint::Tcp { host, port }`, connects to it as [`TcpFactory`](crate::transport::TcpFactory)
/// does (with `TCP_NODELAY`), sends the host as the server name and requires the server's
/// certificate to chain to a trust anchor and to name that host, a DNS name or an IP address.
/// One deadline covers the resolution, the TCP connect and the handshake.
///
/// A server certificate that fails verification (an unknown authority, expired or not yet
/// valid, issued for another name) is [`ErrorKind::Trust`]; a server that refuses the client
/// certificate, or the lack of one, with a certificate alert is
/// [`ErrorKind::Authentication`]. Under TLS 1.3 the server judges the client certificate
/// after the client has finished its handshake, so that refusal arrives with the first
/// `recv`, not at `open`. Session keys are never logged and the keystore's key is never
/// formatted: `Debug` shows nothing of the configuration.
///
/// # Compatibility: docs/parity/transport.md#UTR-TLS
/// Upstream's TLS transport offers TLS 1.3 and 1.2 and verifies the host name unless
/// `ignore-common-name` is set; here the host name is always verified, and trust comes only
/// from the given trust store.
///
/// # Examples
/// A trust store that cannot be read is refused before any I/O:
/// ```
/// use stonecrab::ErrorKind;
/// use stonecrab::transport::{TlsFactory, TlsSettings};
///
/// let error = TlsFactory::new(&TlsSettings::new("/nonexistent/ca.pem")).unwrap_err();
/// assert_eq!(error.kind(), ErrorKind::InvalidConfig);
/// assert!(!error.to_string().contains("/nonexistent"));
/// ```
/// Opening a connection to a device whose certificate a private authority issued:
/// ```no_run
/// use std::time::{Duration, Instant};
/// use bytes::Bytes;
/// use stonecrab::{Deadline, Endpoint, ResourceLimits};
/// use stonecrab::transport::{TlsFactory, TlsSettings, TransportFactory};
///
/// let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
/// runtime.block_on(async {
///     let mut settings = TlsSettings::new("/etc/plc/ca.pem");
///     settings.keystore = Some("/etc/plc/client.pem".into());
///     let factory = TlsFactory::new(&settings)?;
///     let endpoint = Endpoint::Tcp { host: "plc.example".to_owned(), port: 802 };
///     let deadline = Deadline::after(Instant::now(), Duration::from_secs(5))?;
///     let limits = ResourceLimits::builder().build()?;
///     let mut transport = factory.open(&endpoint, limits, deadline).await?;
///     // One Modbus request: read holding register 0 of unit 1.
///     let request = Bytes::from_static(&[0, 1, 0, 0, 0, 6, 1, 3, 0, 0, 0, 1]);
///     transport.send(request, deadline).await?;
///     transport.shutdown(deadline).await?;
///     Ok::<(), Box<dyn std::error::Error>>(())
/// })?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub struct TlsFactory {
    connector: TlsConnector,
}

/// Shows nothing of the TLS configuration, which holds the client's private key.
impl fmt::Debug for TlsFactory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TlsFactory").finish_non_exhaustive()
    }
}

impl TlsFactory {
    /// Read the trust store and the keystore of `settings` and build the factory.
    ///
    /// The files are read once, here, on the caller's thread, with blocking reads of at most
    /// 1 MiB each and no time bound, so the paths must name regular local files (a FIFO or a
    /// stalled network mount blocks the caller); nothing is opened on the network. The
    /// protocol versions are TLS 1.3 and TLS 1.2 and the cipher
    /// suites the defaults of rustls's ring provider, all of them AEAD with an ephemeral key
    /// exchange.
    ///
    /// # Errors
    /// [`ErrorKind::InvalidConfig`] when a file cannot be read or is larger than 1 MiB, when
    /// the trust store holds no PEM certificate or one that is not a usable trust anchor,
    /// and when the keystore holds no certificate, no private key or a key that does not
    /// match its certificate or that the ring provider cannot use. The messages name the
    /// option (`tls.trust-store`, `tls.keystore`) but never a path or file content.
    ///
    /// # Examples
    /// ```
    /// use std::io::Write;
    /// use stonecrab::ErrorKind;
    /// use stonecrab::transport::{TlsFactory, TlsSettings};
    ///
    /// // A file with no certificate in it trusts nothing, which is refused.
    /// let path = std::env::temp_dir().join(format!("stonecrab-doc-{}.pem", std::process::id()));
    /// std::fs::File::create(&path)?.write_all(b"not a certificate\n")?;
    /// let error = TlsFactory::new(&TlsSettings::new(&path)).unwrap_err();
    /// std::fs::remove_file(&path)?;
    /// assert_eq!(error.kind(), ErrorKind::InvalidConfig);
    /// assert!(error.to_string().contains("tls.trust-store"));
    /// # Ok::<(), std::io::Error>(())
    /// ```
    pub fn new(settings: &TlsSettings) -> Result<Self> {
        let roots = trust_anchors(&settings.trust_store)?;
        let builder =
            ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
                .map_err(|error| {
                    PlcError::new(ErrorKind::Internal, "the TLS versions are not supported")
                        .with_source(error)
                })?
                .with_root_certificates(roots);
        let config = match &settings.keystore {
            None => builder.with_no_client_auth(),
            Some(path) => {
                let (chain, key) = identity(path)?;
                builder.with_client_auth_cert(chain, key).map_err(|error| {
                    invalid(
                        KEYSTORE,
                        "holds a key that does not match its certificate or that ring cannot use",
                    )
                    .with_source(error)
                })?
            }
        };
        Ok(Self {
            connector: TlsConnector::from(Arc::new(config)),
        })
    }

    /// The body of `open`, returning the concrete transport.
    async fn open_tls(
        &self,
        endpoint: &Endpoint,
        limits: ResourceLimits,
        deadline: Deadline,
    ) -> Result<TlsTransport> {
        let Endpoint::Tcp { host, port } = endpoint else {
            return Err(Site::TLS_CONNECT.error(
                ErrorKind::InvalidConfig,
                "the tls transport needs an Endpoint::Tcp",
            ));
        };
        let name = ServerName::try_from(host.clone()).map_err(|_refused| {
            Site::TLS_CONNECT.error(
                ErrorKind::InvalidConfig,
                "the host is neither a DNS name nor an IP address a certificate can name",
            )
        })?;
        let stream = guarded(deadline, Site::TLS_CONNECT, async {
            let tcp = resolve_and_connect(host, *port).await?;
            tcp.set_nodelay(true)?;
            self.connector.connect(name, tcp).await
        })
        .await?;
        Ok(TlsTransport {
            stream,
            peer: endpoint.clone(),
            limits,
            usable: true,
            shut_down: false,
        })
    }
}

impl TransportFactory for TlsFactory {
    /// Resolves, connects and completes the TLS handshake within `deadline`, then returns
    /// the transport.
    ///
    /// Checks run in this order: the endpoint is `Endpoint::Tcp` and its host can be a
    /// server name (else `InvalidConfig`), the deadline has not passed, then resolution, the
    /// TCP connect and the handshake run under the deadline, which is the `deadline`
    /// argument. A server certificate that fails verification is `Trust`, a certificate
    /// alert from the server is `Authentication`, any other failure `Disconnected`, and a
    /// deadline that passes `Timeout`.
    ///
    /// # Cancel safety
    /// Cancel-safe: a dropped open closes the socket; no application bytes were sent.
    ///
    /// # Wire effects
    /// A TCP handshake with each address tried, then a TLS handshake (the client
    /// certificate, if the server asks for it and one is set); no application bytes.
    fn open<'a>(
        &'a self,
        endpoint: &'a Endpoint,
        limits: ResourceLimits,
        deadline: Deadline,
    ) -> OpFuture<'a, Box<dyn Transport>> {
        Box::pin(async move {
            let transport: Box<dyn Transport> =
                Box::new(self.open_tls(endpoint, limits, deadline).await?);
            Ok(transport)
        })
    }
}

/// A [`Transport`] over one TLS session on one TCP connection, made by [`TlsFactory`].
///
/// It follows the TCP transport's rules: one owner, no background work, a `send` cut short
/// poisons it, a `recv` loses nothing when dropped, and a peer that closes (with or without
/// a `close_notify`) leaves it unusable.
struct TlsTransport {
    stream: TlsStream<TcpStream>,
    peer: Endpoint,
    limits: ResourceLimits,
    /// False once the peer closed, a write was cut short or a read failed.
    usable: bool,
    /// True once `shutdown` has been called, so a repeat is a no-op.
    shut_down: bool,
}

impl TlsTransport {
    fn ensure_usable(&self, site: Site) -> Result<()> {
        if self.usable {
            Ok(())
        } else {
            Err(site.error(ErrorKind::Disconnected, "transport is no longer usable"))
        }
    }
}

impl Transport for TlsTransport {
    /// Encrypts `bytes` and writes the records to the socket within `deadline`.
    ///
    /// The checks and the poisoning are those of the TCP transport; the frame bound applies
    /// to the plaintext. The records are flushed before the call returns.
    fn send(&mut self, bytes: Bytes, deadline: Deadline) -> OpFuture<'_, ()> {
        Box::pin(async move {
            ensure_before(deadline, Site::TLS_SEND)?;
            self.ensure_usable(Site::TLS_SEND)?;
            let limit = self.limits.max_frame_bytes().get();
            if bytes.len() > limit {
                let detail = format!(
                    "frame of {} bytes exceeds the {limit} byte frame limit",
                    bytes.len()
                );
                return Err(Site::TLS_SEND.error(ErrorKind::ResourceLimit, &detail));
            }
            self.usable = false;
            let stream = &mut self.stream;
            guarded(deadline, Site::TLS_SEND, async {
                stream.write_all(&bytes).await?;
                stream.flush().await
            })
            .await?;
            self.usable = true;
            Ok(())
        })
    }

    /// Appends decrypted bytes to the free space of `buffer` within `deadline`.
    ///
    /// The checks are those of the TCP transport. Records read from the socket stay in the
    /// TLS session until their plaintext is delivered, so a dropped or timed-out call loses
    /// nothing. A `close_notify` from the peer reads `0`; a refusal of the client
    /// certificate that a TLS 1.3 server sends after the handshake is `Authentication`.
    fn recv<'a>(&'a mut self, buffer: &'a mut BytesMut, deadline: Deadline) -> OpFuture<'a, usize> {
        Box::pin(async move {
            ensure_before(deadline, Site::TLS_RECV)?;
            self.ensure_usable(Site::TLS_RECV)?;
            let bound = self.limits.max_buffered_bytes().get();
            let free = bound.saturating_sub(buffer.len());
            if free == 0 {
                let detail = format!("receive buffer already holds the {bound} byte limit");
                return Err(Site::TLS_RECV.error(ErrorKind::ResourceLimit, &detail));
            }
            let mut window = (&mut *buffer).limit(free);
            match guarded(deadline, Site::TLS_RECV, self.stream.read_buf(&mut window)).await {
                Ok(0) => {
                    self.usable = false;
                    Ok(0)
                }
                Ok(read) => Ok(read),
                Err(error) => {
                    self.usable &= error.kind() == ErrorKind::Timeout;
                    Err(error)
                }
            }
        })
    }

    /// Sends `close_notify`, then half-closes the socket, within `deadline`; closed from the
    /// first poll, and a repeat call returns `Ok(())` without I/O.
    fn shutdown(&mut self, deadline: Deadline) -> OpFuture<'_, ()> {
        Box::pin(async move {
            if self.shut_down {
                return Ok(());
            }
            ensure_before(deadline, Site::TLS_SHUTDOWN)?;
            self.shut_down = true;
            self.usable = false;
            guarded(deadline, Site::TLS_SHUTDOWN, self.stream.shutdown()).await
        })
    }

    fn peer(&self) -> &Endpoint {
        &self.peer
    }

    fn limits(&self) -> &ResourceLimits {
        &self.limits
    }
}

/// [`within`], except that a TLS refusal becomes `Trust` or `Authentication` instead of
/// `Disconnected`.
async fn guarded<T>(
    deadline: Deadline,
    site: Site,
    io: impl Future<Output = io::Result<T>>,
) -> Result<T> {
    within(deadline, site, async {
        match io.await {
            Ok(value) => Ok(Ok(value)),
            Err(error) => match refusal(&error) {
                Some(kind) => Ok(Err(site.error(kind, &error.to_string()).with_source(error))),
                None => Err(error),
            },
        }
    })
    .await?
}

/// The error kind of a TLS refusal carried by `error`: `Trust` when the server's certificate
/// failed verification here, `Authentication` when the server refused this client's
/// certificate with a certificate alert. Anything else is no refusal.
fn refusal(error: &io::Error) -> Option<ErrorKind> {
    let tls = error.get_ref()?.downcast_ref::<rustls::Error>()?;
    if matches!(tls, rustls::Error::InvalidCertificate(_)) {
        return Some(ErrorKind::Trust);
    }
    let rustls::Error::AlertReceived(alert) = tls else {
        return None;
    };
    matches!(
        alert,
        AlertDescription::BadCertificate
            | AlertDescription::UnsupportedCertificate
            | AlertDescription::CertificateRevoked
            | AlertDescription::CertificateExpired
            | AlertDescription::CertificateUnknown
            | AlertDescription::UnknownCA
            | AlertDescription::AccessDenied
            | AlertDescription::CertificateRequired
    )
    .then_some(ErrorKind::Authentication)
}

/// The refusal of a TLS option, naming the option and never the path or the content.
fn invalid(option: &str, problem: &str) -> PlcError {
    PlcError::new(ErrorKind::InvalidConfig, format!("{option} {problem}"))
}

/// Read a PEM file of at most [`MAX_PEM_BYTES`].
fn read_pem(path: &Path, option: &str) -> Result<Vec<u8>> {
    let file =
        File::open(path).map_err(|error| invalid(option, "cannot be opened").with_source(error))?;
    let mut bytes = Vec::new();
    file.take(MAX_PEM_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| invalid(option, "cannot be read").with_source(error))?;
    if !u64::try_from(bytes.len()).is_ok_and(|length| length <= MAX_PEM_BYTES) {
        return Err(invalid(option, "is larger than 1 MiB"));
    }
    Ok(bytes)
}

/// The trust anchors of the trust store: every PEM certificate in it, at least one.
fn trust_anchors(path: &Path) -> Result<RootCertStore> {
    let pem = read_pem(path, TRUST_STORE)?;
    let mut roots = RootCertStore::empty();
    for certificate in CertificateDer::pem_slice_iter(&pem) {
        let certificate = certificate.map_err(|error| {
            invalid(TRUST_STORE, "holds a PEM section that cannot be decoded").with_source(error)
        })?;
        roots.add(certificate).map_err(|error| {
            invalid(
                TRUST_STORE,
                "holds a certificate that is not a usable trust anchor",
            )
            .with_source(error)
        })?;
    }
    if roots.is_empty() {
        return Err(invalid(TRUST_STORE, "holds no PEM certificate"));
    }
    Ok(roots)
}

/// The certificate chain and the private key of the keystore. A decoding failure is
/// reported without its source, which could quote the key's text.
fn identity(path: &Path) -> Result<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)> {
    let pem = read_pem(path, KEYSTORE)?;
    let chain = CertificateDer::pem_slice_iter(&pem)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_undecodable| invalid(KEYSTORE, "holds a PEM section that cannot be decoded"))?;
    if chain.is_empty() {
        return Err(invalid(KEYSTORE, "holds no PEM certificate"));
    }
    let key = PrivateKeyDer::from_pem_slice(&pem).map_err(|_missing| {
        invalid(
            KEYSTORE,
            "holds no unencrypted PEM private key (PKCS #8, SEC1 or PKCS #1)",
        )
    })?;
    Ok((chain, key))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::net::TcpListener;

    use super::*;
    use crate::Operation;
    use crate::transport::deadline::now;

    fn factory_without_anchors() -> TlsFactory {
        let config =
            ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
                .expect("versions")
                .with_root_certificates(RootCertStore::empty())
                .with_no_client_auth();
        TlsFactory {
            connector: TlsConnector::from(Arc::new(config)),
        }
    }

    // covers: SPEC mandatory-failure-cases / F2 transport (deadline expiry over the TLS handshake)
    #[tokio::test]
    async fn the_connect_deadline_bounds_a_handshake_the_server_never_answers() {
        // The TCP connection is made on the real clock (paused time does not control socket
        // readiness, decision D21); the handshake then runs under `guarded` on the paused
        // clock, as in `open_tls`. The device reads the first byte of the client's hello and
        // never answers, so the clock moves to the deadline once every task idles.
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("address");
        let (client, accepted) = tokio::join!(TcpStream::connect(address), listener.accept());
        let (client, (mut device, _)) = (client.expect("connect"), accepted.expect("accept"));
        let factory = factory_without_anchors();
        let name = ServerName::try_from("127.0.0.1".to_owned()).expect("an IP name");
        tokio::time::pause();
        let started = tokio::time::Instant::now();
        let deadline = Deadline::after(now(), Duration::from_secs(3)).expect("deadline");
        let mut first = [0_u8; 1];
        let (read, opened) = tokio::join!(
            device.read_exact(&mut first),
            guarded(
                deadline,
                Site::TLS_CONNECT,
                factory.connector.connect(name, client)
            )
        );
        let elapsed = started.elapsed();
        tokio::time::resume();
        read.expect("the device read the hello");
        assert_eq!(first, [0x16], "a TLS handshake record had started");
        let error = opened.expect_err("no ServerHello ever comes");
        assert_eq!(error.kind(), ErrorKind::Timeout, "the deadline passed");
        assert_eq!(
            error.diagnostics().operation,
            Some(Operation::Connect),
            "a connect failure"
        );
        assert!(
            elapsed >= Duration::from_secs(3) && elapsed < Duration::from_secs(4),
            "at the deadline (the timer rounds to the millisecond): {elapsed:?}"
        );
    }

    #[test]
    fn only_certificate_failures_and_certificate_alerts_are_refusals() {
        let wrap = |tls: rustls::Error| io::Error::new(io::ErrorKind::InvalidData, tls);
        let cases = [
            (
                rustls::Error::InvalidCertificate(rustls::CertificateError::UnknownIssuer),
                Some(ErrorKind::Trust),
            ),
            (
                rustls::Error::InvalidCertificate(rustls::CertificateError::Expired),
                Some(ErrorKind::Trust),
            ),
            (
                rustls::Error::InvalidCertificate(rustls::CertificateError::NotValidForName),
                Some(ErrorKind::Trust),
            ),
            (
                rustls::Error::AlertReceived(AlertDescription::CertificateRequired),
                Some(ErrorKind::Authentication),
            ),
            (
                rustls::Error::AlertReceived(AlertDescription::UnknownCA),
                Some(ErrorKind::Authentication),
            ),
            (
                rustls::Error::AlertReceived(AlertDescription::BadCertificate),
                Some(ErrorKind::Authentication),
            ),
            (
                rustls::Error::AlertReceived(AlertDescription::HandshakeFailure),
                None,
            ),
            (
                rustls::Error::AlertReceived(AlertDescription::ProtocolVersion),
                None,
            ),
            (rustls::Error::DecryptError, None),
        ];
        for (tls, want) in cases {
            let shown = format!("{tls:?}");
            assert_eq!(refusal(&wrap(tls)), want, "{shown}");
        }
        assert_eq!(
            refusal(&io::Error::from(io::ErrorKind::ConnectionReset)),
            None,
            "a plain socket failure"
        );
    }

    #[tokio::test]
    async fn a_refusal_keeps_its_kind_through_the_site_and_the_source() {
        let deadline = Deadline::after(now(), Duration::from_secs(5)).expect("deadline");
        let error = guarded(deadline, Site::TLS_CONNECT, async {
            Err::<(), _>(io::Error::new(
                io::ErrorKind::InvalidData,
                rustls::Error::InvalidCertificate(rustls::CertificateError::UnknownIssuer),
            ))
        })
        .await
        .expect_err("refused");
        assert_eq!(error.kind(), ErrorKind::Trust, "certificate failure");
        assert_eq!(
            error.diagnostics().operation,
            Some(Operation::Connect),
            "the site's operation"
        );
        assert!(
            error.to_string().starts_with("tls connect: "),
            "the site's name: {error}"
        );
        let source = std::error::Error::source(&error).expect("the io error is kept");
        assert!(source.downcast_ref::<io::Error>().is_some(), "io source");
        let error = guarded(deadline, Site::TLS_RECV, async {
            Err::<(), _>(io::Error::from(io::ErrorKind::ConnectionReset))
        })
        .await
        .expect_err("reset");
        assert_eq!(error.kind(), ErrorKind::Disconnected, "no refusal");
    }
}
