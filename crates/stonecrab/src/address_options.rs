//! Driver-independent options that influence how protocol address text is parsed.

use crate::{ErrorKind, PlcError, Result};

/// Options a driver consults while parsing address text, carried beside the text.
///
/// Drivers stay stateless factories: the options travel as an argument of address
/// parsing instead of living in driver state (decision D16, ADR-0008). The carrier is
/// [`Clone`] so a caller can take it from a
/// [`ConnectionConfig`](crate::ConnectionConfig) before `connect` consumes the config.
///
/// The only option at alpha is the address convention, which selects how ambiguous
/// address spellings are read. It is derived from the shared connection-string key
/// `address-convention`: every driver maps that key into [`AddressOptionsBuilder`], so
/// the key means the same thing for every scheme. The Modbus driver accepts only the
/// value `plc4x` at alpha and rejects any other name as an invalid address (D16). Core
/// does not know the legal names; it only guarantees the name is a short token with no
/// whitespace or control characters, so a driver can compare it without further
/// normalisation.
///
/// The default carries no convention, which means "none supplied", not "use the
/// driver's preferred one".
///
/// # Compatibility: docs/parity/core.md#CORE-ADDRESS-OPTIONS
/// Upstream accepts the ambiguous bare-digit Modbus forms unconditionally and has no
/// option to select a convention. The carrier is a Stonecrab addition so a driver can
/// accept them only when the caller said which convention applies.
///
/// # Examples
/// ```
/// use stonecrab::AddressOptions;
/// assert_eq!(AddressOptions::default().convention(), None);
/// let options = AddressOptions::builder().convention("plc4x").build()?;
/// assert_eq!(options.convention(), Some("plc4x"));
/// assert_eq!(options.clone(), options);
/// # Ok::<(), stonecrab::PlcError>(())
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct AddressOptions {
    convention: Option<String>,
}

impl AddressOptions {
    /// Longest accepted convention name, in bytes.
    pub const MAX_CONVENTION_BYTES: usize = 64;

    /// Start building options; nothing is validated until
    /// [`build`](AddressOptionsBuilder::build).
    ///
    /// # Examples
    /// ```
    /// use stonecrab::AddressOptions;
    /// let options = AddressOptions::builder().build()?;
    /// assert_eq!(options, AddressOptions::default());
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    #[must_use = "configure and validate the address options before use"]
    pub fn builder() -> AddressOptionsBuilder {
        AddressOptionsBuilder::default()
    }

    /// The address convention name the caller supplied, if any.
    ///
    /// The name is exactly the validated text given to
    /// [`AddressOptionsBuilder::convention`]; core does not lowercase or trim it.
    #[must_use]
    pub fn convention(&self) -> Option<&str> {
        self.convention.as_deref()
    }
}

/// Builder for [`AddressOptions`].
///
/// Setters only record the request; [`build`](Self::build) validates it, matching the
/// other core builders.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
#[must_use]
pub struct AddressOptionsBuilder {
    convention: Option<String>,
}

impl AddressOptionsBuilder {
    /// Select the address convention by name, for example `plc4x`.
    ///
    /// The name must be 1 to [`AddressOptions::MAX_CONVENTION_BYTES`] bytes with no
    /// whitespace or control characters; this is checked by [`build`](Self::build).
    /// A later call replaces an earlier one.
    #[must_use = "builder changes must be retained"]
    pub fn convention(mut self, name: &str) -> Self {
        self.convention = Some(name.to_owned());
        self
    }

    /// Validate the recorded options without performing any I/O.
    ///
    /// # Errors
    /// Returns [`ErrorKind::InvalidConfig`] if the convention name is empty, longer
    /// than [`AddressOptions::MAX_CONVENTION_BYTES`] bytes, or contains whitespace or
    /// control characters. The name is not echoed in the message.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::{AddressOptions, ErrorKind};
    /// let error = AddressOptions::builder().convention("").build().unwrap_err();
    /// assert_eq!(error.kind(), ErrorKind::InvalidConfig);
    /// ```
    #[must_use = "address option validation must be checked"]
    pub fn build(self) -> Result<AddressOptions> {
        if let Some(name) = &self.convention {
            validate_convention(name)?;
        }
        Ok(AddressOptions {
            convention: self.convention,
        })
    }
}

fn validate_convention(name: &str) -> Result<()> {
    let reason = if name.is_empty() {
        "address-convention must not be empty"
    } else if name.len() > AddressOptions::MAX_CONVENTION_BYTES {
        "address-convention exceeds the 64 byte limit"
    } else if name.chars().any(|c| c.is_whitespace() || c.is_control()) {
        "address-convention must not contain whitespace or control characters"
    } else {
        return Ok(());
    };
    Err(PlcError::new(ErrorKind::InvalidConfig, reason))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_and_empty_builder_carry_no_convention() {
        assert_eq!(AddressOptions::default().convention(), None, "default");
        let built = AddressOptions::builder().build().expect("empty is valid");
        assert_eq!(built.convention(), None, "builder default");
        assert_eq!(built, AddressOptions::default(), "equal to Default");
    }

    #[test]
    fn convention_round_trips_clones_and_last_call_wins() {
        let options = AddressOptions::builder()
            .convention("other")
            .convention("plc4x")
            .build()
            .expect("valid name");
        assert_eq!(options.convention(), Some("plc4x"), "last write wins");
        let copy = options.clone();
        assert_eq!(copy, options, "clone is equal");
        assert_ne!(copy, AddressOptions::default(), "differs from default");
        let longest = "a".repeat(AddressOptions::MAX_CONVENTION_BYTES);
        let edge = AddressOptions::builder()
            .convention(&longest)
            .build()
            .expect("exactly the limit is valid");
        assert_eq!(edge.convention(), Some(longest.as_str()), "limit kept");
        assert!(
            format!("{options:?}").contains("plc4x"),
            "debug is derived and names the convention"
        );
        let builder = AddressOptions::builder().convention("plc4x");
        assert!(
            format!("{:?}", builder.clone()).contains("plc4x"),
            "builder debug"
        );
    }

    #[test]
    fn invalid_conventions_are_rejected_without_echoing_them() {
        let too_long = "a".repeat(AddressOptions::MAX_CONVENTION_BYTES + 1);
        for name in [
            "",
            too_long.as_str(),
            "plc 4x",
            " plc4x",
            "plc4x\t",
            "plc4x\n",
            "plc\u{0}4x",
            "plc\u{7f}4x",
            "plc\u{a0}4x",
            "plc\u{2003}4x",
        ] {
            let error = AddressOptions::builder()
                .convention(name)
                .build()
                .expect_err("invalid convention");
            assert_eq!(error.kind(), ErrorKind::InvalidConfig, "typed: {name:?}");
            assert!(
                name.is_empty() || !error.to_string().contains(name),
                "message does not echo the name"
            );
            assert!(
                error.to_string().contains("address-convention"),
                "message names the key"
            );
        }
    }
}
