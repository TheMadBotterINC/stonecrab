//! Out-of-band credential transport and cache reuse, through public APIs.
#![cfg(all(feature = "sim", feature = "testkit"))]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use stonecrab::sim::{SimDevice, SimDriver};
    use stonecrab::testkit::{FakeClock, block_on_with};
    use stonecrab::{
        AddressOptions, Capabilities, Connection, ConnectionCache, ConnectionConfig,
        CredentialIdentity, Credentials, Driver, DriverRegistry, ErrorKind, OpFuture, PlcError,
        ProtocolAddress, Result,
    };

    const URI: &str = "sim://credential-bench";
    type Record = (Option<(String, String)>, String, String, bool);
    struct RecordingDriver {
        driver: SimDriver,
        records: Arc<Mutex<Vec<Record>>>,
    }
    impl Driver for RecordingDriver {
        fn scheme(&self) -> &'static str {
            self.driver.scheme()
        }
        fn capabilities(&self) -> Capabilities {
            self.driver.capabilities()
        }
        fn parse_config(&self, uri: &str) -> Result<ConnectionConfig> {
            self.driver.parse_config(uri)
        }
        fn parse_address(
            &self,
            text: &str,
            options: &AddressOptions,
        ) -> Result<Arc<dyn ProtocolAddress>> {
            self.driver.parse_address(text, options)
        }
        fn connect(&self, config: ConnectionConfig) -> OpFuture<'_, Box<dyn Connection>> {
            let credentials = config.credentials().map(|secret| {
                let c = secret.expose();
                (c.username.clone(), c.password.expose().clone())
            });
            self.records.lock().unwrap().push((
                credentials,
                format!("{config:?}"),
                config.redacted_display(),
                config.driver_options().is_some(),
            ));
            self.driver.connect(config)
        }
    }
    fn registry(clock: &FakeClock) -> (Arc<DriverRegistry>, Arc<Mutex<Vec<Record>>>) {
        let records = Arc::new(Mutex::new(Vec::new()));
        let driver = SimDriver::new()
            .with_device("credential-bench", SimDevice::new())
            .unwrap()
            .with_clock(Arc::new(clock.clone()));
        let mut registry = DriverRegistry::new();
        registry
            .register(Arc::new(RecordingDriver {
                driver,
                records: Arc::clone(&records),
            }))
            .unwrap();
        (Arc::new(registry), records)
    }

    // covers: CORE-REGISTRY / CORE-CONFIG-SHAPE
    #[test]
    fn registry_connect_with_keeps_secrets_out_of_the_uri() {
        let clock = FakeClock::default();
        let (registry, records) = registry(&clock);
        let uri = format!("{URI}?latency-ms=3");
        let (_, parsed) = registry.parse(&uri).unwrap();
        assert!(parsed.credentials().is_none());
        let connection = block_on_with(
            &clock,
            registry.connect_with(
                &uri,
                Credentials::new("operator-sentinel", "password-sentinel"),
            ),
        )
        .unwrap();
        assert_eq!(
            stonecrab::ConnectionUri::parse(&uri).unwrap().to_string(),
            uri
        );
        let seen = records.lock().unwrap();
        let (credentials, debug, display, options) = seen.first().unwrap();
        assert_eq!(
            credentials.as_ref(),
            Some(&(
                "operator-sentinel".to_owned(),
                "password-sentinel".to_owned()
            ))
        );
        for text in [debug, display] {
            assert!(!text.contains("operator-sentinel"));
            assert!(!text.contains("password-sentinel"));
        }
        assert!(!*options, "sim keeps latency in its custom endpoint");
        assert!(debug.contains("latency-ms=3"));
        drop(seen);
        block_on_with(&clock, connection.close()).unwrap();
        assert_eq!(
            block_on_with(
                &clock,
                registry.connect_with("broken", Credentials::new("x", "password-sentinel"))
            )
            .err()
            .unwrap()
            .kind(),
            ErrorKind::InvalidConfig
        );
        assert_eq!(
            block_on_with(
                &clock,
                registry.connect_with("missing://host", Credentials::new("x", "password-sentinel"))
            )
            .err()
            .unwrap()
            .kind(),
            ErrorKind::Unsupported(stonecrab::Operation::Connect)
        );
    }

    #[test]
    fn certificate_identity_is_additive_and_redacts_both_paths() {
        let credentials = Credentials::x509("certificate-sentinel.der", "key-sentinel.pem");
        let CredentialIdentity::X509 {
            certificate_file,
            private_key_file,
        } = credentials.identity()
        else {
            panic!("certificate identity")
        };
        assert_eq!(
            certificate_file,
            std::path::Path::new("certificate-sentinel.der")
        );
        assert_eq!(private_key_file, std::path::Path::new("key-sentinel.pem"));
        assert_eq!(credentials.username, "");
        assert_eq!(credentials.password.expose(), "");
        for text in [
            format!("{credentials:?}"),
            format!("{:?}", credentials.identity()),
        ] {
            assert!(!text.contains("certificate-sentinel"));
            assert!(!text.contains("key-sentinel"));
            assert!(text.contains("REDACTED"));
        }
        let mut username = Credentials::new("old", "password-sentinel");
        username.username = "new".to_owned();
        assert!(matches!(
            username.identity(),
            CredentialIdentity::UserNamePassword
        ));
        assert_eq!(username.username, "new");
        assert!(format!("{username:?}").contains("UserNamePassword"));
    }

    // covers: TOOL-CC-CACHE
    #[test]
    fn cache_provider_is_resolved_only_for_new_connections_and_keys_stay_secret_free() {
        let clock = FakeClock::default();
        let (registry, records) = registry(&clock);
        let calls = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&calls);
        let builder = ConnectionCache::builder(registry, Arc::new(clock.clone()))
            .credential_provider(move |uri| {
                assert_eq!(uri.scheme(), "sim");
                assert_eq!(uri.authority(), "credential-bench");
                count.fetch_add(1, Ordering::SeqCst);
                Ok(Some(Credentials::new(
                    "operator-sentinel",
                    "password-sentinel",
                )))
            });
        assert!(!format!("{builder:?}").contains("password-sentinel"));
        let cache = builder.build().unwrap();
        let first = block_on_with(&clock, cache.get(URI)).unwrap();
        drop(first);
        let second = block_on_with(&clock, cache.get(URI)).unwrap();
        drop(second);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(records.lock().unwrap().len(), 1);
        assert_eq!(cache.connection_strings(), [URI]);
        assert!(!format!("{cache:?}").contains("password-sentinel"));
        assert!(block_on_with(&clock, cache.remove(URI)).unwrap());
        let replacement = block_on_with(&clock, cache.get(URI)).unwrap();
        drop(replacement);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(records.lock().unwrap().len(), 2);
        block_on_with(&clock, cache.close()).unwrap();
    }

    #[test]
    fn cache_provider_errors_do_not_fall_back_to_anonymous_and_none_keeps_the_config() {
        let clock = FakeClock::default();
        let (registry, records) = registry(&clock);
        let calls = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&calls);
        let cache = ConnectionCache::builder(registry, Arc::new(clock.clone()))
            .max_idle_time(Duration::from_secs(1))
            .credential_provider(move |_| {
                if count.fetch_add(1, Ordering::SeqCst) == 0 {
                    return Err(PlcError::new(
                        ErrorKind::AccessDenied,
                        "credential provider unavailable",
                    ));
                }
                Ok(None)
            })
            .build()
            .unwrap();
        assert_eq!(
            block_on_with(&clock, cache.get("missing://host"))
                .err()
                .unwrap()
                .kind(),
            ErrorKind::Unsupported(stonecrab::Operation::Connect)
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            block_on_with(&clock, cache.get(URI)).err().unwrap().kind(),
            ErrorKind::AccessDenied
        );
        assert_eq!(records.lock().unwrap().len(), 0);
        let lease = block_on_with(&clock, cache.get(URI)).unwrap();
        drop(lease);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert!(records.lock().unwrap().first().unwrap().0.is_none());
        block_on_with(&clock, cache.close()).unwrap();
    }
}
