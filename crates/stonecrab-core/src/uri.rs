//! The connection-string grammar: `scheme(:transport)?://authority(?k=v(&k=v)*)?`.
//!
//! Parsing only splits and validates text. It resolves no host name, opens nothing and
//! gives no meaning to a scheme, a transport code or an option; a driver does that with
//! the parts it is handed (ADR-0008).

use std::collections::BTreeSet;
use std::fmt;

use crate::{ErrorKind, PlcError, Result};

/// A parsed connection string: scheme, optional transport code, authority and options.
///
/// The grammar is `scheme(:transport)?://authority(?k=v(&k=v)*)?`:
///
/// - `scheme` and `transport` are non-empty runs of `a-z`, `0-9` and `-`.
/// - `authority` is everything after `://` up to the first `?`, and must not be empty.
///   Its inner structure (`host:port`, a simulator device name, ...) belongs to the
///   driver.
/// - After the first `?` come `key=value` pairs separated by `&`. Every pair needs an
///   `=` and a non-empty key; the key ends at the first `=`, so a value may contain
///   further `=` characters. An empty value (`key=`) is allowed. Keys are compared
///   exactly (case-sensitive) and may not repeat.
///
/// Options keep the order in which they were written, in a `Vec`. Because duplicate
/// keys are rejected, order carries no meaning for a driver that looks options up by
/// key; keeping it makes [`Display`](fmt::Display) reproduce the caller's text and
/// keeps diagnostics in the order the caller wrote them.
///
/// The type is meant to hold no credentials, so its `Debug` output shows everything.
/// [`parse`](Self::parse) rejects the standard `user:password@host` form: any `@` before
/// the first `/` of the authority is an error. An `@` after that `/` is path text and is
/// kept verbatim, so do not put secrets there. Pass a user name and password through
/// [`Credentials`](crate::Credentials) instead.
///
/// Deliberate differences from upstream, all of them rejections:
///
/// - Percent-encoding is rejected: any `%` anywhere in the text. Upstream passes the
///   authority and the parameter string through unchanged and leaves decoding to
///   whichever consumer reads them, so there is no single decoded form to match.
/// - Credentials in the authority are rejected (see above); upstream passes the
///   authority through.
/// - An empty scheme, an empty transport code (`scheme:://...`), an empty parameter
///   string (`...?`) and empty pairs (`a=1&&b=2`, a trailing `&`) are rejected;
///   upstream's regular expression admits all four.
/// - Control characters and whitespace anywhere are rejected, and the whole text is
///   limited to [`MAX_BYTES`](Self::MAX_BYTES) bytes.
///
/// # Compatibility: docs/parity/core.md#CORE-URI-GRAMMAR
/// Upstream `DriverBase.URI_PATTERN` is
/// `^(?<protocolCode>[a-z0-9\-]*)(:(?<transportCode>[a-z0-9\-]*))?://(?<transportConfig>[^?]*)(\?(?<paramString>.*))?`.
/// This type accepts a strict subset of that language: the same character class for
/// scheme and transport, the same split of the authority at the first `?`, and the
/// differences listed above.
///
/// # Examples
/// ```
/// use stonecrab_core::ConnectionUri;
/// let uri = ConnectionUri::parse("modbus-tcp:tcp://192.0.2.10:502?unit-id=3&timeout=500")?;
/// assert_eq!(uri.scheme(), "modbus-tcp");
/// assert_eq!(uri.transport(), Some("tcp"));
/// assert_eq!(uri.authority(), "192.0.2.10:502");
/// assert_eq!(uri.option("unit-id"), Some("3"));
/// assert_eq!(uri.to_string(), "modbus-tcp:tcp://192.0.2.10:502?unit-id=3&timeout=500");
/// # Ok::<(), stonecrab_core::PlcError>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionUri {
    scheme: String,
    transport: Option<String>,
    authority: String,
    options: Vec<(String, String)>,
}

impl ConnectionUri {
    /// Longest accepted connection string, in bytes.
    pub const MAX_BYTES: usize = 4096;

    /// Parse and validate `text`.
    ///
    /// The checks run in this order: length, control characters and whitespace, `%`,
    /// the `://` separator, scheme and transport characters, the authority, then each
    /// option pair. No message contains the authority or the query text, so a rejected
    /// string cannot leak a secret that was pasted into it by mistake.
    ///
    /// # Errors
    /// Returns [`ErrorKind::InvalidConfig`] if the text is longer than
    /// [`MAX_BYTES`](Self::MAX_BYTES), contains a control character, whitespace or `%`,
    /// has no `://`, has an empty scheme, an empty transport code or a character
    /// outside `a-z0-9-` in either, has an empty authority or one with credentials (an
    /// `@` before its first `/`), or has an option pair without `=`, with an empty key
    /// or with a key that already appeared.
    ///
    /// # Compatibility: docs/parity/core.md#CORE-URI-GRAMMAR
    /// See the type documentation for the grammar and its differences from upstream.
    ///
    /// # Examples
    /// ```
    /// use stonecrab_core::{ConnectionUri, ErrorKind};
    /// let uri = ConnectionUri::parse("sim://bench")?;
    /// assert_eq!((uri.scheme(), uri.transport(), uri.authority()), ("sim", None, "bench"));
    /// assert!(uri.options().is_empty());
    ///
    /// let error = ConnectionUri::parse("sim://operator:hunter2@bench").unwrap_err();
    /// assert_eq!(error.kind(), ErrorKind::InvalidConfig);
    /// assert!(!error.to_string().contains("hunter2"));
    /// # Ok::<(), stonecrab_core::PlcError>(())
    /// ```
    #[must_use = "the parse result must be checked"]
    pub fn parse(text: &str) -> Result<Self> {
        if text.len() > Self::MAX_BYTES {
            return Err(invalid("connection string is longer than 4096 bytes"));
        }
        if text.chars().any(|c| c.is_control() || c.is_whitespace()) {
            return Err(invalid(
                "connection string contains a control character or whitespace",
            ));
        }
        if text.contains('%') {
            return Err(invalid("percent-encoding is not supported"));
        }
        let Some((head, rest)) = text.split_once("://") else {
            return Err(invalid("connection string has no \"://\" separator"));
        };
        let (scheme, transport) = match head.split_once(':') {
            Some((scheme, transport)) => (scheme, Some(transport)),
            None => (head, None),
        };
        check_code(
            scheme,
            "scheme must not be empty",
            "scheme may contain only a-z, 0-9 and '-'",
        )?;
        if let Some(transport) = transport {
            check_code(
                transport,
                "transport must not be empty",
                "transport may contain only a-z, 0-9 and '-'",
            )?;
        }
        let (authority, query) = match rest.split_once('?') {
            Some((authority, query)) => (authority, Some(query)),
            None => (rest, None),
        };
        if authority.is_empty() {
            return Err(invalid("authority must not be empty"));
        }
        if authority
            .split_once('/')
            .map_or(authority, |(host, _path)| host)
            .contains('@')
        {
            return Err(invalid(
                "credentials in the authority are not supported; use Credentials",
            ));
        }
        let options = match query {
            Some(query) => parse_options(query)?,
            None => Vec::new(),
        };
        Ok(Self {
            scheme: scheme.to_owned(),
            transport: transport.map(str::to_owned),
            authority: authority.to_owned(),
            options,
        })
    }

    /// The driver scheme, such as `modbus-tcp`; never empty.
    #[must_use]
    pub fn scheme(&self) -> &str {
        &self.scheme
    }

    /// The transport code after the scheme's `:`, such as `tcp`; `None` if the text had
    /// none. When present it is never empty.
    #[must_use]
    pub fn transport(&self) -> Option<&str> {
        self.transport.as_deref()
    }

    /// The text between `://` and the first `?` (or the end); never empty and never
    /// holding credentials.
    #[must_use]
    pub fn authority(&self) -> &str {
        &self.authority
    }

    /// The options as `(key, value)` pairs in the order they were written. Keys are
    /// unique and non-empty; a value may be empty.
    #[must_use]
    pub fn options(&self) -> &[(String, String)] {
        &self.options
    }

    /// The value of option `key`, compared exactly (case-sensitive), or `None` if it
    /// was not given. `Some("")` means the key was given with an empty value.
    ///
    /// # Examples
    /// ```
    /// use stonecrab_core::ConnectionUri;
    /// let uri = ConnectionUri::parse("sim://bench?mode=fast&tag=")?;
    /// assert_eq!(uri.option("mode"), Some("fast"));
    /// assert_eq!(uri.option("tag"), Some(""));
    /// assert_eq!(uri.option("Mode"), None);
    /// # Ok::<(), stonecrab_core::PlcError>(())
    /// ```
    #[must_use]
    pub fn option(&self, key: &str) -> Option<&str> {
        self.options
            .iter()
            .find(|(candidate, _)| candidate == key)
            .map(|(_, value)| value.as_str())
    }
}

/// Renders the canonical text: `scheme`, `:transport` when present, `://`, the
/// authority, and `?k=v&k=v` when there are options, in their original order.
///
/// Parsing the output gives back an equal value.
///
/// # Examples
/// ```
/// use stonecrab_core::ConnectionUri;
/// let uri = ConnectionUri::parse("modbus-tcp://plc.example:502?unit-id=1")?;
/// assert_eq!(uri.to_string(), "modbus-tcp://plc.example:502?unit-id=1");
/// assert_eq!(ConnectionUri::parse(&uri.to_string())?, uri);
/// # Ok::<(), stonecrab_core::PlcError>(())
/// ```
impl fmt::Display for ConnectionUri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.scheme)?;
        if let Some(transport) = &self.transport {
            write!(f, ":{transport}")?;
        }
        write!(f, "://{}", self.authority)?;
        for (index, (key, value)) in self.options.iter().enumerate() {
            f.write_str(if index == 0 { "?" } else { "&" })?;
            write!(f, "{key}={value}")?;
        }
        Ok(())
    }
}

/// Check one code (`scheme` or `transport`): non-empty, `a-z`, `0-9` and `-` only.
fn check_code(code: &str, empty: &'static str, bad_chars: &'static str) -> Result<()> {
    if code.is_empty() {
        return Err(invalid(empty));
    }
    if !code
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err(invalid(bad_chars));
    }
    Ok(())
}

/// Split a query into ordered `(key, value)` pairs, refusing empty pairs, pairs without
/// `=`, empty keys and repeated keys.
fn parse_options(query: &str) -> Result<Vec<(String, String)>> {
    let mut seen = BTreeSet::new();
    let mut options = Vec::new();
    for pair in query.split('&') {
        let Some((key, value)) = pair.split_once('=') else {
            return Err(invalid("option pair has no '='"));
        };
        if key.is_empty() {
            return Err(invalid("option key must not be empty"));
        }
        if !seen.insert(key) {
            return Err(invalid("option key is repeated"));
        }
        options.push((key.to_owned(), value.to_owned()));
    }
    Ok(options)
}

fn invalid(message: &'static str) -> PlcError {
    PlcError::new(ErrorKind::InvalidConfig, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rejected(text: &str) -> PlcError {
        let error = ConnectionUri::parse(text).expect_err("text must be rejected");
        assert_eq!(error.kind(), ErrorKind::InvalidConfig, "kind for {text:?}");
        error
    }

    fn pair(key: &str, value: &str) -> (String, String) {
        (key.to_owned(), value.to_owned())
    }

    // covers: CODING_STANDARDS 13.7 zero length
    #[test]
    fn uri_empty_input_is_rejected() {
        rejected("");
    }

    // covers: CODING_STANDARDS 13.7 one byte short of a valid form
    #[test]
    fn uri_forms_one_byte_short_are_rejected() {
        for text in ["modbus-tcp:/", "modbus-tcp:", "modbus-tcp", "://x"] {
            rejected(text);
        }
        let error = rejected("modbus-tcp://");
        assert!(
            error.to_string().contains("authority"),
            "names the authority"
        );
        let error = rejected("modbus-tcp://?a=1");
        assert!(
            error.to_string().contains("authority"),
            "options need an authority"
        );
    }

    // covers: CODING_STANDARDS 13.7 exact minimal form
    #[test]
    fn uri_minimal_form_is_accepted() {
        let uri = ConnectionUri::parse("a://b").expect("minimal form");
        assert_eq!(uri.scheme(), "a", "scheme");
        assert_eq!(uri.transport(), None, "no transport");
        assert_eq!(uri.authority(), "b", "authority");
        assert!(uri.options().is_empty(), "no options");
        assert_eq!(uri.to_string(), "a://b", "canonical text");
    }

    // covers: CODING_STANDARDS 13.7 one byte long
    #[test]
    fn uri_trailing_separators_are_rejected() {
        for text in [
            "sim://bench?",
            "sim://bench?a=1&",
            "sim://bench?&a=1",
            "sim://bench?a=1&&b=2",
        ] {
            let error = rejected(text);
            assert!(error.to_string().contains("'='"), "empty pair: {text}");
        }
    }

    // covers: CODING_STANDARDS 13.7 maximum-plus-one length
    #[test]
    fn uri_over_4096_bytes_is_rejected() {
        let prefix = "sim://";
        let at_limit = format!(
            "{prefix}{}",
            "a".repeat(ConnectionUri::MAX_BYTES - prefix.len())
        );
        assert_eq!(at_limit.len(), 4096, "fixture is at the limit");
        let uri = ConnectionUri::parse(&at_limit).expect("4096 bytes are accepted");
        assert_eq!(uri.authority().len(), 4090, "authority kept whole");
        let over = format!("{at_limit}a");
        let error = rejected(&over);
        assert!(error.to_string().contains("4096"), "names the bound");
        // The bound is checked first: an over-long string with another fault reports it.
        let error = rejected(&format!("{over}%"));
        assert!(
            error.to_string().contains("4096"),
            "length wins over percent"
        );
    }

    // covers: CODING_STANDARDS 13.7 every invalid discriminator (scheme and transport)
    #[test]
    fn uri_invalid_scheme_and_transport_are_rejected() {
        for text in [
            "://bench",
            "Sim://bench",
            "sim_x://bench",
            "si.m://bench",
            "sim:Tcp://bench",
            "sim:tc_p://bench",
            "sim::tcp://bench",
            "sim:tcp:x://bench",
            "sim:://bench",
            "\u{e9}://bench",
        ] {
            rejected(text);
        }
        assert!(
            rejected("://bench").to_string().contains("scheme"),
            "empty scheme"
        );
        assert!(
            rejected("sim:://bench").to_string().contains("transport"),
            "empty transport"
        );
        assert!(
            rejected("sim:Tcp://bench")
                .to_string()
                .contains("transport"),
            "bad transport"
        );
        let uri = ConnectionUri::parse("a-1:b-2://bench").expect("digits and hyphens");
        assert_eq!(
            (uri.scheme(), uri.transport()),
            ("a-1", Some("b-2")),
            "codes kept"
        );
    }

    // covers: CODING_STANDARDS 13.7 every invalid discriminator (credential marker)
    #[test]
    fn uri_credentials_in_authority_are_rejected() {
        for text in [
            "sim://operator:hunter2@bench",
            "sim://@bench",
            "sim://bench@",
            "sim://user@host/path",
            "sim://bench@/path?a=1",
        ] {
            let error = rejected(text);
            let shown = format!("{error} {error:?}");
            assert!(
                !shown.contains("hunter2") && !shown.contains("operator"),
                "no echo"
            );
            assert!(
                shown.contains("credentials"),
                "explains the rejection: {text}"
            );
        }
        let uri = ConnectionUri::parse("sim://host/path@x").expect("@ after the first slash");
        assert_eq!(uri.authority(), "host/path@x", "path kept verbatim");
        let uri = ConnectionUri::parse("sim://host?mail=a@b").expect("@ in a value");
        assert_eq!(uri.option("mail"), Some("a@b"), "value kept");
    }

    // covers: CODING_STANDARDS 13.7 every invalid discriminator (percent sign)
    #[test]
    fn uri_percent_encoding_is_rejected() {
        for text in [
            "sim://host%20name",
            "sim://host?a=%41",
            "sim://host?%41=a",
            "sim%3a://host",
            "sim://host?a=%",
            "%",
        ] {
            let error = rejected(text);
            assert!(error.to_string().contains("percent"), "explains: {text}");
        }
    }

    // covers: CODING_STANDARDS 13.7 every invalid discriminator (repeated option key)
    #[test]
    fn uri_duplicate_key_is_rejected() {
        let error = rejected("sim://bench?a=1&b=2&a=3");
        assert!(error.to_string().contains("repeated"), "explains");
        rejected("sim://bench?a=&a=");
        let uri = ConnectionUri::parse("sim://bench?a=1&A=2").expect("keys are case-sensitive");
        assert_eq!(uri.options(), [pair("a", "1"), pair("A", "2")], "both kept");
    }

    // covers: CODING_STANDARDS 13.7 every invalid discriminator (empty option key)
    #[test]
    fn uri_empty_key_is_rejected() {
        let error = rejected("sim://bench?=1");
        assert!(error.to_string().contains("empty"), "explains");
        rejected("sim://bench?a=1&=2");
        rejected("sim://bench?=");
    }

    // covers: CODING_STANDARDS 13.7 every invalid discriminator (option pair without separator)
    #[test]
    fn uri_pair_without_equals_is_rejected() {
        let error = rejected("sim://bench?secret-token");
        assert!(
            !error.to_string().contains("secret-token"),
            "query not echoed"
        );
        rejected("sim://bench?a=1&b");
    }

    // covers: CODING_STANDARDS 13.7 every invalid discriminator (control and whitespace bytes)
    #[test]
    fn uri_control_characters_and_whitespace_are_rejected() {
        for text in [
            " sim://bench",
            "sim://bench ",
            "sim://be nch",
            "sim://bench?a=1 &b=2",
            "sim://bench?a=\t1",
            "sim://bench\n",
            "sim://bench\r?a=1",
            "sim://bench\0",
            "sim://bench\u{7f}",
            "sim://bench\u{85}",
            "sim://bench\u{a0}",
            "sim://bench\u{2028}",
        ] {
            let error = rejected(text);
            assert!(
                error.to_string().contains("whitespace"),
                "explains: {text:?}"
            );
        }
    }

    #[test]
    fn uri_options_keep_order_values_and_empty_values() {
        let uri = ConnectionUri::parse("modbus-tcp:tcp://[::1]:502?z=1&a=&m=x=y&q=?&k=a/b:c")
            .expect("valid");
        assert_eq!(uri.scheme(), "modbus-tcp", "scheme");
        assert_eq!(uri.transport(), Some("tcp"), "transport");
        assert_eq!(uri.authority(), "[::1]:502", "authority");
        assert_eq!(
            uri.options(),
            [
                pair("z", "1"),
                pair("a", ""),
                pair("m", "x=y"),
                pair("q", "?"),
                pair("k", "a/b:c")
            ],
            "written order, empty value, '=' and '?' inside values"
        );
        assert_eq!(uri.option("a"), Some(""), "empty value found");
        assert_eq!(
            uri.option("m"),
            Some("x=y"),
            "value splits at the first '='"
        );
        assert_eq!(uri.option("missing"), None, "absent key");
        assert_eq!(
            uri.to_string(),
            "modbus-tcp:tcp://[::1]:502?z=1&a=&m=x=y&q=?&k=a/b:c"
        );
    }

    #[test]
    fn uri_authority_stops_at_the_first_question_mark_only() {
        let uri = ConnectionUri::parse("sim://a:b/c?d=e").expect("valid");
        assert_eq!(
            uri.authority(),
            "a:b/c",
            "slash and colon stay in the authority"
        );
        let uri = ConnectionUri::parse("sim://x://y&z=1").expect("authority holds anything else");
        assert_eq!(uri.authority(), "x://y&z=1", "second separator kept");
        let uri = ConnectionUri::parse("sim://\u{e9}h\u{f4}te").expect("non-ASCII authority");
        assert_eq!(uri.authority(), "\u{e9}h\u{f4}te", "kept verbatim");
    }

    #[test]
    fn uri_debug_and_clone_show_the_parts() {
        let uri = ConnectionUri::parse("sim://bench?a=1").expect("valid");
        assert_eq!(uri.clone(), uri, "clone is equal");
        let debug = format!("{uri:?}");
        assert!(
            debug.contains("bench") && debug.contains("\"a\""),
            "debug shows parts"
        );
    }

    #[test]
    fn uri_display_parses_back_to_an_equal_value() {
        for text in ["a://b", "a:b://c", "a://b?c=d", "a:b://c/d?e=f&g=&h=i=j"] {
            let uri = ConnectionUri::parse(text).expect("valid");
            assert_eq!(uri.to_string(), text, "already canonical");
            assert_eq!(
                ConnectionUri::parse(&uri.to_string()).expect("reparse"),
                uri,
                "equal"
            );
        }
    }
}
