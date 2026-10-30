//! `OpcuaOptions`: the driver-owned connection parameters of the `opcua` scheme, and the
//! endpoint of its connection string.
//!
//! Everything here is crate-private until the driver arrives (slice S91). A key and a value
//! are read exactly: names are case-sensitive, and an error message names a key (shortened)
//! or a fixed reason, never a value, the host or the endpoint path.

use crate::{DriverOptions, Endpoint, ErrorKind, PlcError, Result};

/// `security-policy`.
const SECURITY_POLICY: &str = "security-policy";
/// `message-security`.
const MESSAGE_SECURITY: &str = "message-security";

/// The most key characters an error message repeats.
const KEY_ECHO_LIMIT: usize = 32;

/// The longest endpoint URL a Hello message may carry, in bytes (OPC UA Part 6).
const MAX_ENDPOINT_URL_BYTES: usize = 4096;

/// The three keys the framework fills from the connection string; upstream lists them as
/// parameters but a user never supplies them (`OPCUA-CFG-FRAMEWORK`).
const FRAMEWORK_KEYS: [&str; 3] = ["protocol-code", "transport-code", "transport-config"];

/// The name upstream gave `tls.verify` before its sense was reversed (`OPCUA-QUIRK-TLS-NAMES`).
const LEGACY_VERIFY: &str = "insecure-certificate-verification";

/// The OPC UA security policy of the secure channel (`security-policy`).
///
/// The two deprecated SHA-1 policies of upstream, `Basic128Rsa15` and `Basic256`, are
/// recognised by name and refused (decision E16), so no value stands for them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SecurityPolicy {
    /// `NONE`: no signing, no encryption.
    None,
    /// `Basic256Sha256`, the default.
    Basic256Sha256,
    /// `Aes128_Sha256_RsaOaep`.
    Aes128Sha256RsaOaep,
    /// `Aes256_Sha256_RsaPss`.
    Aes256Sha256RsaPss,
}

impl SecurityPolicy {
    /// The policy a name stands for, case-sensitive.
    fn from_name(name: &str) -> Result<Self> {
        match name {
            "NONE" => Ok(Self::None),
            "Basic256Sha256" => Ok(Self::Basic256Sha256),
            "Aes128_Sha256_RsaOaep" => Ok(Self::Aes128Sha256RsaOaep),
            "Aes256_Sha256_RsaPss" => Ok(Self::Aes256Sha256RsaPss),
            "Basic128Rsa15" | "Basic256" => Err(invalid(format!(
                "{SECURITY_POLICY} Basic128Rsa15 and Basic256 are deprecated SHA-1 policies and \
                 are not offered; use Basic256Sha256, Aes128_Sha256_RsaOaep or \
                 Aes256_Sha256_RsaPss"
            ))),
            _ => Err(invalid(format!(
                "{SECURITY_POLICY} must be NONE, Basic256Sha256, Aes128_Sha256_RsaOaep or \
                 Aes256_Sha256_RsaPss"
            ))),
        }
    }

    /// The name `security-policy` takes for this policy.
    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::None => "NONE",
            Self::Basic256Sha256 => "Basic256Sha256",
            Self::Aes128Sha256RsaOaep => "Aes128_Sha256_RsaOaep",
            Self::Aes256Sha256RsaPss => "Aes256_Sha256_RsaPss",
        }
    }

    /// Whether the policy signs or encrypts, which is every policy but `NONE`.
    pub(super) const fn is_secure(self) -> bool {
        !matches!(self, Self::None)
    }
}

/// The message security mode of the secure channel (`message-security`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MessageSecurity {
    /// `NONE`.
    None,
    /// `SIGN`.
    Sign,
    /// `SIGN_ENCRYPT`, the default.
    SignEncrypt,
}

impl MessageSecurity {
    /// The mode a name stands for, case-sensitive.
    fn from_name(name: &str) -> Result<Self> {
        match name {
            "NONE" => Ok(Self::None),
            "SIGN" => Ok(Self::Sign),
            "SIGN_ENCRYPT" => Ok(Self::SignEncrypt),
            _ => Err(invalid(format!(
                "{MESSAGE_SECURITY} must be NONE, SIGN or SIGN_ENCRYPT"
            ))),
        }
    }

    /// The name `message-security` takes for this mode.
    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::None => "NONE",
            Self::Sign => "SIGN",
            Self::SignEncrypt => "SIGN_ENCRYPT",
        }
    }
}

/// The connection parameters of the `opcua` driver, parsed and validated.
///
/// `parse_config` reads them from the connection string and stores them in the
/// `ConnectionConfig` through `ConnectionConfigBuilder::driver_options`; `connect` reads them
/// back with `ConnectionConfig::driver_options_as`. A key the connection string does not
/// name keeps its default.
///
/// | Field | Source | Default |
/// | --- | --- | --- |
/// | `endpoint_url` | the authority: `opc.tcp://host:port` and the path | none, the port is required |
/// | `security_policy` | `security-policy` | `Basic256Sha256` |
/// | `message_security` | `message-security` | `SIGN_ENCRYPT` |
///
/// `message_security` is the mode as configured. With policy `NONE` the channel ignores it
/// (`OPCUA-MODE-POLICY-NONE`, slice S90); a secure policy with mode `NONE` is refused here.
///
/// Its `Debug` output carries no secret: it holds none. It does show the endpoint path, as
/// every connection string text does, so a path must not carry one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct OpcuaOptions {
    /// The URL Hello carries and endpoint matching starts from: `opc.tcp://host:port` and the
    /// path of the connection string, verbatim.
    pub(super) endpoint_url: String,
    /// The security policy (`security-policy`).
    pub(super) security_policy: SecurityPolicy,
    /// The message security mode (`message-security`).
    pub(super) message_security: MessageSecurity,
}

impl DriverOptions for OpcuaOptions {}

impl OpcuaOptions {
    /// Read the query parameters of a connection string for the endpoint `endpoint_url`.
    ///
    /// The core grammar has already refused duplicate and empty keys, so each key appears
    /// once. Every key but `security-policy` and `message-security` is refused, which
    /// includes the keys the framework fills, the credentials a URI must not carry and the
    /// upstream names that later slices implement or that have gone.
    pub(super) fn from_parameters(
        endpoint_url: String,
        parameters: &[(String, String)],
    ) -> Result<Self> {
        let mut security_policy = SecurityPolicy::Basic256Sha256;
        let mut message_security = MessageSecurity::SignEncrypt;
        for (key, value) in parameters {
            match key.as_str() {
                SECURITY_POLICY => security_policy = SecurityPolicy::from_name(value)?,
                MESSAGE_SECURITY => message_security = MessageSecurity::from_name(value)?,
                other => return Err(unknown_key(other)),
            }
        }
        if security_policy.is_secure() && message_security == MessageSecurity::None {
            return Err(invalid(format!(
                "{MESSAGE_SECURITY}=NONE needs {SECURITY_POLICY}=NONE: a secure policy signs \
                 or encrypts"
            )));
        }
        Ok(Self {
            endpoint_url,
            security_policy,
            message_security,
        })
    }
}

/// The endpoint a `host:port[/path]` authority names, and the endpoint URL Hello carries.
///
/// The host is a name, an IPv4 address or an IPv6 address in brackets. The port is required:
/// upstream defines no default and a defaulted port hides a typo (decision E22). The path is
/// everything from the first `/`, kept verbatim.
pub(super) fn endpoint(authority: &str) -> Result<(Endpoint, String)> {
    let (address, path) = match authority.split_once('/') {
        Some((address, rest)) => (address, Some(rest)),
        None => (authority, None),
    };
    let (host, port) = if let Some(bracketed) = address.strip_prefix('[') {
        let Some((host, rest)) = bracketed.split_once(']') else {
            return Err(invalid("an IPv6 host in brackets needs a closing bracket"));
        };
        if host.parse::<std::net::Ipv6Addr>().is_err() {
            return Err(invalid("the bracketed host is not an IPv6 address"));
        }
        let Some(port) = rest.strip_prefix(':') else {
            return Err(if rest.is_empty() {
                missing_port()
            } else {
                invalid("only a port may follow the bracketed host")
            });
        };
        (host, port)
    } else {
        let Some((host, port)) = address.split_once(':') else {
            return Err(missing_port());
        };
        if !is_name(host) {
            return Err(invalid(
                "the host is empty or not a name or IPv4 address (IPv6 needs brackets)",
            ));
        }
        (host, port)
    };
    let endpoint = Endpoint::Tcp {
        host: host.to_owned(),
        port: parse_port(port)?,
    };
    let path = path.map_or_else(String::new, |rest| format!("/{rest}"));
    let url = format!("opc.tcp://{endpoint}{path}");
    if url.len() > MAX_ENDPOINT_URL_BYTES {
        return Err(invalid("the endpoint URL is longer than 4096 bytes"));
    }
    Ok((endpoint, url))
}

/// The refusal of an authority without a port.
fn missing_port() -> PlcError {
    invalid("the port is required, as in opcua://host:port: there is no default port")
}

/// A port: decimal digits only, 1 to 65535.
fn parse_port(text: &str) -> Result<u16> {
    let digits = !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit());
    match text.parse::<u16>() {
        Ok(port) if digits && port != 0 => Ok(port),
        _ => Err(invalid("the port is a whole number from 1 to 65535")),
    }
}

/// A host name or IPv4 address: ASCII letters, digits and `-`, `_`, `.`; not empty.
fn is_name(host: &str) -> bool {
    host.bytes().any(|byte| byte.is_ascii_alphanumeric())
        && host
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

/// The refusal of a key the driver does not know; the key is shortened, nothing else of the
/// connection string is repeated.
fn unknown_key(key: &str) -> PlcError {
    let mut chars = key.chars();
    let mut shown: String = chars.by_ref().take(KEY_ECHO_LIMIT).collect();
    if chars.next().is_some() {
        shown.push_str("...");
    }
    let hint = if FRAMEWORK_KEYS.contains(&key) {
        " (the scheme, transport and authority of the connection string set it)"
    } else if key == "username" || key == "password" {
        " (credentials never travel in the connection string; pass Credentials)"
    } else if key == LEGACY_VERIFY {
        " (the old upstream name; tls.verify replaced it with the opposite sense)"
    } else if key.starts_with("tls.") {
        " (the tls. certificate options are not offered yet)"
    } else {
        ""
    };
    invalid(format!(
        "unknown connection parameter '{shown}'{hint}; the parameters are {SECURITY_POLICY} \
         and {MESSAGE_SECURITY}"
    ))
}

fn invalid(message: impl Into<String>) -> PlcError {
    PlcError::new(ErrorKind::InvalidConfig, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "opc.tcp://host:4840";

    fn parse(pairs: &[(&str, &str)]) -> Result<OpcuaOptions> {
        let owned: Vec<(String, String)> = pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect();
        OpcuaOptions::from_parameters(URL.to_owned(), &owned)
    }

    fn options(pairs: &[(&str, &str)]) -> OpcuaOptions {
        parse(pairs).expect("valid parameters")
    }

    fn refusal(pairs: &[(&str, &str)]) -> PlcError {
        parse(pairs).expect_err("the parameters must be refused")
    }

    fn refused(authority: &str) -> String {
        let error = endpoint(authority).expect_err("the authority must be refused");
        assert_eq!(error.kind(), ErrorKind::InvalidConfig, "{authority}");
        error.to_string()
    }

    // covers: OPCUA-CFG-SECURITY-POLICY the default policy is Basic256Sha256 with SIGN_ENCRYPT
    #[test]
    fn opcua_config_security_policy_default_is_basic256sha256() {
        let defaults = options(&[]);
        assert_eq!(defaults.security_policy, SecurityPolicy::Basic256Sha256);
        assert_eq!(defaults.message_security, MessageSecurity::SignEncrypt);
        assert_eq!(defaults.endpoint_url, URL);
        assert!(defaults.security_policy.is_secure());
    }

    #[test]
    fn opcua_config_security_policy_parses_the_four_supported_names() {
        for (name, expected) in [
            ("NONE", SecurityPolicy::None),
            ("Basic256Sha256", SecurityPolicy::Basic256Sha256),
            ("Aes128_Sha256_RsaOaep", SecurityPolicy::Aes128Sha256RsaOaep),
            ("Aes256_Sha256_RsaPss", SecurityPolicy::Aes256Sha256RsaPss),
        ] {
            let parsed = options(&[(SECURITY_POLICY, name), (MESSAGE_SECURITY, "SIGN")]);
            assert_eq!(parsed.security_policy, expected, "{name}");
            assert_eq!(expected.name(), name, "the name round-trips");
            assert_eq!(expected.is_secure(), name != "NONE", "{name}");
        }
    }

    // covers: OPCUA-POLICY-BASIC128RSA15 / OPCUA-POLICY-BASIC256 refused with the reason
    #[test]
    fn opcua_basic128rsa15_is_refused_as_deprecated() {
        let error = refusal(&[(SECURITY_POLICY, "Basic128Rsa15")]);
        assert_eq!(error.kind(), ErrorKind::InvalidConfig);
        assert!(error.to_string().contains("deprecated"), "{error}");
    }

    #[test]
    fn opcua_basic256_is_refused_as_deprecated() {
        let error = refusal(&[(SECURITY_POLICY, "Basic256")]);
        assert_eq!(error.kind(), ErrorKind::InvalidConfig);
        assert!(error.to_string().contains("deprecated"), "{error}");
    }

    #[test]
    fn opcua_config_security_policy_unknown_name_is_invalid_config() {
        for value in [
            "",
            "none",
            "None",
            "basic256sha256",
            "BASIC256SHA256",
            "Basic256Sha256 ",
            "Aes128Sha256RsaOaep",
            "Aes256_Sha256_RsaPss2",
            "http://opcfoundation.org/UA/SecurityPolicy#None",
        ] {
            let error = refusal(&[(SECURITY_POLICY, value)]);
            assert_eq!(error.kind(), ErrorKind::InvalidConfig, "{value:?}");
            assert!(
                error.to_string().contains(SECURITY_POLICY),
                "names the key: {error}"
            );
            assert!(
                !error.to_string().contains("deprecated"),
                "only the two SHA-1 names are deprecated: {error}"
            );
        }
    }

    // covers: OPCUA-POLICY-ECC an ECC policy is an unknown name
    #[test]
    fn opcua_config_security_policy_ecc_name_is_invalid_config() {
        for value in [
            "ECC_nistP256",
            "ECC_nistP384",
            "ECC_brainpoolP256r1",
            "ECC_curve25519",
        ] {
            let error = refusal(&[(SECURITY_POLICY, value)]);
            assert_eq!(error.kind(), ErrorKind::InvalidConfig, "{value}");
        }
    }

    // covers: OPCUA-CFG-MESSAGE-SECURITY the three names and the default
    #[test]
    fn opcua_config_message_security_default_is_sign_encrypt() {
        assert_eq!(options(&[]).message_security, MessageSecurity::SignEncrypt);
        assert_eq!(
            options(&[(SECURITY_POLICY, "NONE")]).message_security,
            MessageSecurity::SignEncrypt,
            "policy NONE keeps the configured mode (the channel ignores it)"
        );
    }

    #[test]
    fn opcua_config_message_security_accepts_the_three_names() {
        for (name, expected, with_secure_policy) in [
            ("NONE", MessageSecurity::None, false),
            ("SIGN", MessageSecurity::Sign, true),
            ("SIGN_ENCRYPT", MessageSecurity::SignEncrypt, true),
        ] {
            assert_eq!(expected.name(), name, "the name round-trips");
            let none = options(&[(SECURITY_POLICY, "NONE"), (MESSAGE_SECURITY, name)]);
            assert_eq!(none.message_security, expected, "{name} with policy NONE");
            if with_secure_policy {
                let secure = options(&[(MESSAGE_SECURITY, name)]);
                assert_eq!(secure.message_security, expected, "{name} with the default");
            }
        }
    }

    #[test]
    fn opcua_config_message_security_unknown_name_is_invalid_config() {
        for value in [
            "",
            "sign",
            "Sign",
            "SIGN-ENCRYPT",
            "SIGNANDENCRYPT",
            "ENCRYPT",
        ] {
            let error = refusal(&[(MESSAGE_SECURITY, value)]);
            assert_eq!(error.kind(), ErrorKind::InvalidConfig, "{value:?}");
            assert!(
                error.to_string().contains(MESSAGE_SECURITY),
                "names the key: {error}"
            );
        }
    }

    // covers: OPCUA-CFG-MESSAGE-SECURITY / OPCUA-DIV-SECURE-MODE-NONE a secure policy with message-security NONE is refused
    #[test]
    fn opcua_a_secure_policy_with_message_security_none_is_invalid_config() {
        for policy in [
            "Basic256Sha256",
            "Aes128_Sha256_RsaOaep",
            "Aes256_Sha256_RsaPss",
        ] {
            let error = refusal(&[(SECURITY_POLICY, policy), (MESSAGE_SECURITY, "NONE")]);
            assert_eq!(error.kind(), ErrorKind::InvalidConfig, "{policy}");
            assert!(error.to_string().contains(MESSAGE_SECURITY), "{error}");
        }
        let defaulted = refusal(&[(MESSAGE_SECURITY, "NONE")]);
        assert_eq!(
            defaulted.kind(),
            ErrorKind::InvalidConfig,
            "the default policy is secure"
        );
        let reversed = refusal(&[
            (MESSAGE_SECURITY, "NONE"),
            (SECURITY_POLICY, "Basic256Sha256"),
        ]);
        assert_eq!(
            reversed.kind(),
            ErrorKind::InvalidConfig,
            "key order does not matter"
        );
        assert_eq!(
            options(&[(SECURITY_POLICY, "NONE"), (MESSAGE_SECURITY, "NONE")]).security_policy,
            SecurityPolicy::None,
            "policy NONE with mode NONE is the unprotected channel"
        );
    }

    // covers: OPCUA-CFG-FRAMEWORK the framework fills these, a user cannot
    #[test]
    fn opcua_config_framework_keys_are_not_user_options() {
        for key in ["protocol-code", "transport-code", "transport-config"] {
            let error = refusal(&[(key, "opcua")]);
            assert_eq!(error.kind(), ErrorKind::InvalidConfig, "{key}");
            assert!(error.to_string().contains(&format!("'{key}'")), "{error}");
            assert!(
                error.to_string().contains("connection string set it"),
                "the hint says where the value comes from: {error}"
            );
        }
    }

    // covers: OPCUA-CFG-UNKNOWN / OPCUA-DIV-UNKNOWN-KEYS unknown keys are rejected
    #[test]
    fn opcua_config_unknown_key_is_rejected() {
        let error = refusal(&[("surprise", "1")]);
        assert_eq!(error.kind(), ErrorKind::InvalidConfig);
        assert!(error.to_string().contains("'surprise'"), "{error}");
        assert!(
            error
                .to_string()
                .contains("security-policy and message-security"),
            "lists the parameters: {error}"
        );
        let after_a_known_key = refusal(&[(SECURITY_POLICY, "NONE"), ("surprise", "1")]);
        assert_eq!(after_a_known_key.kind(), ErrorKind::InvalidConfig);
        assert!(
            after_a_known_key.to_string().contains("'surprise'"),
            "{after_a_known_key}"
        );
        for message in [error.to_string(), after_a_known_key.to_string()] {
            assert!(
                !message.contains("deprecated") && !message.contains("credentials"),
                "a plain unknown key gets no hint: {message}"
            );
        }
    }

    // covers: OPCUA-TRANSPORT-ALLOW-UNSUPPORTED / OPCUA-CFG-GENERATED-KEY-SIZE deferred options
    #[test]
    fn opcua_config_allow_unsupported_transport_is_rejected() {
        let error = refusal(&[("allow-unsupported-transport", "true")]);
        assert_eq!(error.kind(), ErrorKind::InvalidConfig);
        assert!(
            error.to_string().contains("'allow-unsupported-transport'"),
            "{error}"
        );
    }

    #[test]
    fn opcua_config_generated_key_size_is_an_unknown_key() {
        let error = refusal(&[("generated-key-size", "2048")]);
        assert_eq!(error.kind(), ErrorKind::InvalidConfig);
        assert!(
            error.to_string().contains("'generated-key-size'"),
            "{error}"
        );
    }

    #[test]
    fn opcua_config_misspelt_security_policy_key_is_rejected() {
        for key in [
            "security_policy",
            "securitypolicy",
            "Security-Policy",
            "security-policies",
            "message_security",
            "messagesecurity",
            "Message-Security",
        ] {
            let error = refusal(&[(key, "NONE")]);
            assert_eq!(error.kind(), ErrorKind::InvalidConfig, "{key}");
            assert!(error.to_string().contains(&format!("'{key}'")), "{error}");
        }
    }

    // covers: OPCUA-QUIRK-TLS-NAMES the legacy name is an unknown key, the tls. names are held back
    #[test]
    fn opcua_config_insecure_certificate_verification_is_an_unknown_key() {
        let error = refusal(&[(LEGACY_VERIFY, "true")]);
        assert_eq!(error.kind(), ErrorKind::InvalidConfig);
        assert!(error.to_string().contains("opposite sense"), "{error}");
        assert_eq!(
            error.to_string().matches('\'').count(),
            2,
            "only the key is quoted, so a reader can tell it from the text around it: {error}"
        );
    }

    #[test]
    fn opcua_config_tls_names_are_not_offered_yet() {
        for key in [
            "tls.keystore",
            "tls.keystore-type",
            "tls.keystore-password",
            "tls.trust-store",
            "tls.trust-store-type",
            "tls.trust-store-password",
            "tls.verify",
        ] {
            let error = refusal(&[(key, "x")]);
            assert_eq!(error.kind(), ErrorKind::InvalidConfig, "{key}");
            assert!(error.to_string().contains("not offered yet"), "{error}");
        }
    }

    fn credential_key_is_refused(key: &str) {
        let error = refusal(&[(key, "hunter2")]);
        assert_eq!(error.kind(), ErrorKind::InvalidConfig, "{key}");
        assert!(error.to_string().contains("pass Credentials"), "{error}");
        assert!(!error.to_string().contains("hunter2"), "{error}");
    }

    // covers: OPCUA-CFG-USERNAME the URI-key half (decision E22); S99 closes the row
    #[test]
    fn opcua_config_username_key_is_invalid_config() {
        credential_key_is_refused("username");
    }

    // covers: OPCUA-CFG-PASSWORD the URI-key half (decision E22); S99 closes the row
    #[test]
    fn opcua_config_password_key_is_invalid_config() {
        credential_key_is_refused("password");
    }

    #[test]
    fn an_unknown_key_is_shortened() {
        let long = "k".repeat(200);
        let shortened = refusal(&[(long.as_str(), "1")]).to_string();
        assert!(shortened.contains("..."), "{shortened}");
        assert!(!shortened.contains(&long), "the key is shortened");
        let exact = "k".repeat(KEY_ECHO_LIMIT);
        let whole = refusal(&[(exact.as_str(), "1")]).to_string();
        assert!(whole.contains(&exact) && !whole.contains("..."), "{whole}");
    }

    // covers: OPCUA-TRANSPORT-PORT the port is required (decision E22)
    #[test]
    fn opcua_an_endpoint_without_a_port_is_refused() {
        for authority in ["host", "host/path", "[::1]", "[::1]/path", "192.0.2.10"] {
            let message = refused(authority);
            assert!(
                message.contains("port is required"),
                "{authority}: {message}"
            );
        }
    }

    // covers: OPCUA-TRANSPORT-PORT the port is 1 to 65535
    #[test]
    fn opcua_an_endpoint_port_outside_1_to_65535_is_refused() {
        for authority in [
            "host:0",
            "host:65536",
            "host:99999999999999999999",
            "host:",
            "host:+4840",
            "host:-1",
            "host:4840x",
            "host:0x12e8",
            "host:48 40",
            "host:4840:1",
            "[::1]:0",
            "[::1]:65536",
            "[::1]:",
            "host:/path",
        ] {
            refused(authority);
        }
        assert!(endpoint("host:1").is_ok(), "the lowest port");
        assert!(endpoint("host:65535").is_ok(), "the highest port");
    }

    // covers: OPCUA-TRANSPORT-ENDPOINT-URL opc.tcp://host:port and the path
    #[test]
    fn opcua_endpoint_url_is_opc_tcp_host_port_and_path() {
        for (authority, host, port, url) in [
            ("plc:4840", "plc", 4840, "opc.tcp://plc:4840"),
            (
                "192.0.2.10:48010/Server",
                "192.0.2.10",
                48010,
                "opc.tcp://192.0.2.10:48010/Server",
            ),
            (
                "plc-1.line_2:1",
                "plc-1.line_2",
                1,
                "opc.tcp://plc-1.line_2:1",
            ),
            ("[::1]:4840", "::1", 4840, "opc.tcp://[::1]:4840"),
            (
                "[2001:db8::10]:4840/ua/server",
                "2001:db8::10",
                4840,
                "opc.tcp://[2001:db8::10]:4840/ua/server",
            ),
            (
                "[::ffff:192.0.2.1]:4840",
                "::ffff:192.0.2.1",
                4840,
                "opc.tcp://[::ffff:192.0.2.1]:4840",
            ),
        ] {
            let (parsed, text) = endpoint(authority).expect(authority);
            assert_eq!(
                parsed,
                Endpoint::Tcp {
                    host: host.to_owned(),
                    port
                },
                "{authority}"
            );
            assert_eq!(text, url, "{authority}");
        }
    }

    // covers: OPCUA-TRANSPORT-ENDPOINT-URL the path is kept as written
    #[test]
    fn opcua_an_endpoint_path_is_kept_verbatim() {
        for path in [
            "/",
            "//",
            "/Server",
            "/Server/",
            "/OPCUA/SimulationServer",
            "/UPPER/lower/MiXed",
            "/a:b",
            "/a@b",
            "/a#b",
            "/a;b=c",
            "/a/./b/../c",
            "/\u{00e9}\u{4e2d}",
        ] {
            let (_, url) = endpoint(&format!("plc:4840{path}")).expect(path);
            assert_eq!(url, format!("opc.tcp://plc:4840{path}"), "{path}");
        }
        let (_, no_path) = endpoint("plc:4840").expect("no path");
        assert_eq!(no_path, "opc.tcp://plc:4840", "no path adds no slash");
    }

    #[test]
    fn opcua_an_endpoint_url_over_4096_bytes_is_refused() {
        // "opc.tcp://" is 10 bytes and "h:1" is 3, so a path of 4083 bytes makes 4096.
        let fits = format!("h:1/{}", "p".repeat(4082));
        assert_eq!(endpoint(&fits).expect("exactly 4096").1.len(), 4096);
        let over = format!("h:1/{}", "p".repeat(4083));
        assert!(refused(&over).contains("4096"));
    }

    #[test]
    fn opcua_a_bad_host_is_invalid_config() {
        for authority in [
            ":4840",
            "-:4840",
            ".:4840",
            "ho st:4840",
            "ho[st:4840",
            "::1:4840",
            "[::1:4840",
            "[]:4840",
            "[::1]x",
            "[::1]x:4840",
            "[host]:4840",
            "[1.2.3.4]:4840",
            "[::g]:4840",
            "[1:2:3:4:5:6:7:8:9]:4840",
            "/path",
            "@host:4840",
        ] {
            refused(authority);
        }
        assert!(refused("[::1]x").contains("only a port may follow"));
        assert!(refused("[::1]x:4840").contains("only a port may follow"));
        assert!(refused("[::1:4840").contains("closing bracket"));
        assert!(refused("[host]:4840").contains("not an IPv6 address"));
        assert!(refused("ho st:4840").contains("not a name or IPv4 address"));
    }

    #[test]
    fn a_refusal_never_repeats_the_host_or_the_path() {
        for authority in ["secret-host:0/secret-path", "secret-host/secret-path"] {
            let message = refused(authority);
            assert!(!message.contains("secret"), "{message}");
        }
    }
}
